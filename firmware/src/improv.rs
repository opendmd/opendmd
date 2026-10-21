//! Improv Serial on the console UART: how a browser installer (ESP Web
//! Tools, `install/` on the site and `flasher/`) recognises a board that
//! already runs OpenDMD and gives it WiFi details over USB.
//!
//! The protocol is `opendmd_core::improv` (framing, RPC parsing, the baud
//! sniffer: host-tested). This is the glue, and it costs no task and no
//! heap while nobody talks to it:
//!
//! - **Receiving** is [`poll`], called by the log ring's drain every 5 ms
//!   on CPU0's timer service task (`logring.rs`). It reads UART0's
//!   registers directly — the receive FIFO's count, the error flags, the
//!   FIFO — the way IDF's own `uart_ll` does, without installing the UART
//!   driver (whose buffers, queue and interrupt would cost heap for good).
//!   The FIFO holds 128 bytes, 11 ms at 115200 baud, so 5 ms polls lose
//!   nothing; the parser lives in RTC slow memory like the ring.
//! - **Sending**: every frame is a record in the log ring (`logring::raw`),
//!   so it goes out whole, between log lines, which is what the host's
//!   reader needs, and nothing waits for the UART.
//! - **The baud.** ESP Web Tools speaks Improv at 115200 only (its
//!   `port.open({baudRate: 115200})` in 10.4.0), and the console runs at
//!   921600 (`sdkconfig.defaults`). Bytes at an eighth of the rate arrive as
//!   framing errors and junk, which [`BaudSniffer`] recognises; then the
//!   console switches to 115200 until the next restart, the host's next
//!   request (it repeats it every second) is understood, and the installer's
//!   "Logs & console" is readable too. A 921600 terminal never triggers it.
//! - **Quick answers** (current state, device info) come from here, out of
//!   atomics the network worker keeps current ([`set_network`]), so they
//!   never wait behind a WiFi connect. **Scans and WiFi settings** go to the
//!   network worker (`NetCommand::Improv`), which owns the radio and sends
//!   its own answers ([`send_scan`], [`provisioned`], [`provision_failed`]).
//!
//! What the installer shows for us: "Connected to `<hostname>`", "OpenDMD
//! `<version>` (ESP32)", Install or Update by the manifest's version,
//! "Visit device" (our URL), "Change Wi-Fi" or "Connect to Wi-Fi", and
//! after a fresh install the WiFi form straight away.

use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use esp_idf_sys as sys;
use log::info;
use opendmd_core::improv::{self as imp, BaudSniffer, Error, Event, Parser, Rpc, State};

use crate::chan as mpsc;
use crate::http::NetCommand;
use crate::logring;
use crate::wifi::ScannedAp;

/// What the device-info answer calls the firmware. ESP Web Tools compares
/// it with the manifest's `name` (the Makefile's `fw-release`) to offer
/// Update rather than Install: they must stay the same.
pub const FIRMWARE_NAME: &str = "OpenDMD";
const CHIP: &str = "ESP32";
/// The rate ESP Web Tools opens the port at.
const IMPROV_BAUD: u32 = 115_200;
/// The most networks a scan answer lists (strongest first).
const MAX_SCAN: usize = 16;

// UART0's registers (ESP-IDF v5.3 `soc/esp32/include/soc/uart_reg.h`).
const UART0: usize = 0x3FF4_0000;
/// The FIFO through the APB address: reads (IDF's `uart_ll_read_rxfifo`).
const FIFO_APB: usize = UART0;
/// The FIFO through the AHB address: writes (IDF's `uart_ll_write_txfifo`).
const FIFO_AHB: usize = 0x6000_0000;
const INT_RAW: usize = UART0 + 0x04;
const INT_CLR: usize = UART0 + 0x10;
const STATUS: usize = UART0 + 0x1C;
const MEM_RX_STATUS: usize = UART0 + 0x60;
/// Parity error, framing error, break: what a too-slow sender produces.
const ERR_BITS: u32 = (1 << 2) | (1 << 3) | (1 << 7);
/// Receive FIFO overflow: cleared, but not taken as a wrong baud (a fast
/// terminal pasting at the right rate can overflow it between polls).
const OVF_BIT: u32 = 1 << 4;
/// The TX FIFO's size.
const TX_FIFO: u32 = 128;

/// A job for the network worker.
pub enum Job {
    /// List the networks in range ([`send_scan`]).
    Scan { at: Instant },
    /// Join this network, save it, answer ([`provisioned`] /
    /// [`provision_failed`]).
    Provision {
        ssid: String,
        password: String,
        at: Instant,
    },
}

struct Ctx {
    hostname: String,
    version: &'static str,
    net: mpsc::Sender<NetCommand>,
}

static CTX: OnceLock<Ctx> = OnceLock::new();

/// [`poll`]'s own state: touched only by the drain (one task), after
/// [`init`] wrote it.
struct Rx {
    parser: Parser,
    sniffer: BaudSniffer,
    /// The first poll throws away whatever arrived before the drain ran
    /// (boot noise), so it cannot switch the baud.
    primed: bool,
}

// RTC slow memory, like the log ring: no heap. Written by `init` before
// the drain can run, then the drain's alone.
#[link_section = ".rtc_noinit"]
static mut RX: MaybeUninit<Rx> = MaybeUninit::uninit();
static READY: AtomicBool = AtomicBool::new(false);

/// The network as the worker last published it: 0 down, 1 on a network,
/// 2 the setup network; and the station's IPv4 address (network order).
static NET: AtomicU8 = AtomicU8::new(0);
static IP: AtomicU32 = AtomicU32::new(0);
/// A WiFi-settings RPC is being tried.
static PROVISIONING: AtomicBool = AtomicBool::new(false);
/// The console was switched to [`IMPROV_BAUD`].
static SWITCHED: AtomicBool = AtomicBool::new(false);

/// Set up before `logring::start` (the drain calls [`poll`] from then on).
/// `hostname` is the device name the installer shows; `net` reaches the
/// network worker (commands queue until it runs).
pub fn init(hostname: &str, net: mpsc::Sender<NetCommand>) {
    let _ = CTX.set(Ctx {
        hostname: hostname.to_string(),
        version: crate::state::firmware_version(),
        net,
    });
    // SAFETY: the drain is not running yet (READY is false), so nothing
    // else touches RX.
    unsafe {
        (&raw mut RX).write(MaybeUninit::new(Rx {
            parser: Parser::new(),
            sniffer: BaudSniffer::new(),
            primed: false,
        }));
    }
    READY.store(true, Ordering::Release);
}

/// The network worker's view (`net_worker::publish`): `wifi` is
/// `Status::wifi` (`sta`, `ap`, `disconnected`), `ip` the address, if any.
pub fn set_network(wifi: &str, ip: &str) {
    let ip = parse_ipv4(ip).map_or(0, u32::from_ne_bytes);
    IP.store(ip, Ordering::Relaxed);
    NET.store(
        match wifi {
            "sta" => 1,
            "ap" => 2,
            _ => 0,
        },
        Ordering::Release,
    );
}

/// `a.b.c.d` as octets (what `Ipv4Addr`'s `Display` wrote), without
/// std's address parser.
pub fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = s.split('.');
    for o in &mut out {
        *o = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// The console was switched to 115200 baud for a browser installer.
pub fn switched() -> bool {
    SWITCHED.load(Ordering::Relaxed)
}

fn reg(addr: usize) -> u32 {
    // SAFETY: a 32-bit read of a UART0 register, which is always mapped.
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

fn set_reg(addr: usize, v: u32) {
    // SAFETY: a 32-bit write to a UART0 register.
    unsafe { core::ptr::write_volatile(addr as *mut u32, v) }
}

/// Bytes waiting in the receive FIFO. IDF's `uart_ll_get_rxfifo_len` for
/// this chip: the count register is unreliable when the FIFO is read
/// through the APB address, so the length comes from the FIFO's read and
/// write pointers.
fn rx_len() -> usize {
    let cnt = reg(STATUS) & 0xFF;
    let mem = reg(MEM_RX_STATUS);
    let rd = (mem >> 2) & 0x7FF;
    let wr = (mem >> 13) & 0x7FF;
    (if wr > rd {
        wr - rd
    } else if wr < rd {
        wr + 128 - rd
    } else if cnt > 0 {
        128
    } else {
        0
    }) as usize
}

fn rx_byte() -> u8 {
    reg(FIFO_APB) as u8
}

/// Write `bytes` to the console UART as they are: no `\n` → `\r\n` (the
/// VFS console would add one, which breaks a binary frame). Busy-waits on
/// the TX FIFO like the VFS write does. The log ring's drain calls it for
/// raw records (the only writer, so nothing interleaves).
pub fn uart_raw(bytes: &[u8]) {
    for &b in bytes {
        while (reg(STATUS) >> 16) & 0xFF >= TX_FIFO - 1 {
            core::hint::spin_loop();
        }
        set_reg(FIFO_AHB, u32::from(b));
    }
}

/// One look at the UART, on the drain (CPU0's timer service task, every
/// 5 ms). Cheap when idle: three register reads.
pub fn poll() {
    if !READY.load(Ordering::Acquire) {
        return;
    }
    // SAFETY: only the drain calls this (one task), and `init` wrote RX
    // before READY was set.
    let rx = unsafe { &mut *(&raw mut RX).cast::<Rx>() };
    let raw = reg(INT_RAW);
    let err = raw & ERR_BITS != 0;
    if raw & (ERR_BITS | OVF_BIT) != 0 {
        set_reg(INT_CLR, raw & (ERR_BITS | OVF_BIT));
    }
    let mut n = rx_len();
    if !rx.primed {
        rx.primed = true;
        for _ in 0..n {
            rx_byte();
        }
        return;
    }
    if !SWITCHED.load(Ordering::Relaxed)
        && sys::CONFIG_ESP_CONSOLE_UART_BAUDRATE != IMPROV_BAUD
        && rx.sniffer.poll(n, err)
    {
        for _ in 0..n {
            rx_byte();
        }
        rx.parser.reset();
        switch_baud();
        return;
    }
    while n > 0 {
        n -= 1;
        if let Some(ev) = rx.parser.push(rx_byte()) {
            handle(ev);
        }
    }
}

/// Over to 115200 for good (until restart): the junk sent at 921600 so far
/// ended mid-line for the host, so a newline first.
fn switch_baud() {
    // SAFETY: sets UART0's clock divider; the driver is not installed and
    // the function needs no driver (it takes the HAL's spinlock only).
    unsafe { sys::uart_set_baudrate(0, IMPROV_BAUD) };
    SWITCHED.store(true, Ordering::Relaxed);
    logring::raw(1, &mut |out| {
        out[0] = b'\n';
        1
    });
    info!("console: a host is talking at {IMPROV_BAUD} baud (a browser installer, Improv Serial): the console is at {IMPROV_BAUD} until the next restart");
}

fn handle(ev: Event<'_>) {
    match ev {
        Event::BadChecksum => send_error(Error::InvalidRpc),
        Event::Frame {
            kind: imp::TYPE_RPC,
            data,
        } => match imp::parse_rpc(data) {
            Err(e) => send_error(e),
            Ok(Rpc::CurrentState) => send_current_state(),
            Ok(Rpc::DeviceInfo) => send_info(),
            Ok(Rpc::Scan) => {
                if !queue(Job::Scan { at: Instant::now() }) {
                    send_scan(&[]);
                }
            }
            Ok(Rpc::WifiSettings { ssid, password }) => {
                if PROVISIONING.swap(true, Ordering::AcqRel) {
                    // One at a time (the host serialises its RPCs anyway).
                    send_state(State::Provisioning);
                    return;
                }
                info!("improv: WiFi details for '{ssid}' received over USB; trying them");
                send_error(Error::None);
                send_state(State::Provisioning);
                let job = Job::Provision {
                    ssid: ssid.to_string(),
                    password: password.to_string(),
                    at: Instant::now(),
                };
                if !queue(job) {
                    provision_failed(Error::UnableToConnect);
                }
            }
            Ok(Rpc::Other(_)) => send_error(Error::UnknownRpc),
        },
        // Only the host's RPCs come this way; anything else is not for us.
        Event::Frame { .. } => {}
    }
}

fn queue(job: Job) -> bool {
    CTX.get()
        .is_some_and(|c| c.net.send(NetCommand::Improv(job)).is_ok())
}

/// The station address, if the device is on a network.
fn station_ip() -> Option<[u8; 4]> {
    let ip = IP.load(Ordering::Relaxed);
    (NET.load(Ordering::Acquire) == 1 && ip != 0).then(|| ip.to_ne_bytes())
}

fn send_state(state: State) {
    logring::raw(imp::STATE_FRAME, &mut |out| imp::encode_state(state, out));
}

fn send_error(error: Error) {
    logring::raw(imp::STATE_FRAME, &mut |out| imp::encode_error(error, out));
}

/// An RPC result with the device's URL (after a current-state of
/// Provisioned, or a successful provisioning).
fn send_url(cmd: u8, ip: [u8; 4]) {
    let mut url = [0u8; 24];
    let n = imp::url_for_ip(ip, &mut url);
    // `url_for_ip` writes ASCII only.
    let url = core::str::from_utf8(&url[..n]).unwrap_or("");
    let strings = [url];
    logring::raw(imp::rpc_result_len(&strings), &mut |out| {
        imp::encode_rpc_result(cmd, &strings, out)
    });
}

fn send_current_state() {
    if PROVISIONING.load(Ordering::Acquire) {
        send_state(State::Provisioning);
    } else if let Some(ip) = station_ip() {
        send_state(State::Provisioned);
        send_url(imp::CMD_CURRENT_STATE, ip);
    } else {
        send_state(State::Ready);
    }
}

fn send_info() {
    let Some(c) = CTX.get() else {
        return;
    };
    let strings = [FIRMWARE_NAME, c.version, CHIP, c.hostname.as_str()];
    logring::raw(imp::rpc_result_len(&strings), &mut |out| {
        imp::encode_rpc_result(imp::CMD_DEVICE_INFO, &strings, out)
    });
}

/// The answer to a scan (network worker): one result per network,
/// strongest first, then the empty result that ends the list.
pub fn send_scan(aps: &[ScannedAp]) {
    for ap in aps.iter().take(MAX_SCAN) {
        let rssi = ap.rssi.to_string();
        let strings = [
            ap.ssid.as_str(),
            rssi.as_str(),
            if ap.secure { "YES" } else { "NO" },
        ];
        logring::raw(imp::rpc_result_len(&strings), &mut |out| {
            imp::encode_rpc_result(imp::CMD_SCAN, &strings, out)
        });
    }
    logring::raw(imp::rpc_result_len(&[]), &mut |out| {
        imp::encode_rpc_result(imp::CMD_SCAN, &[], out)
    });
}

/// The WiFi details worked and are saved (network worker): Provisioned,
/// then the URL for "Visit device".
pub fn provisioned(ip: [u8; 4]) {
    PROVISIONING.store(false, Ordering::Release);
    send_state(State::Provisioned);
    send_url(imp::CMD_WIFI_SETTINGS, ip);
}

/// They did not (network worker): the error, then back to Ready.
pub fn provision_failed(error: Error) {
    PROVISIONING.store(false, Ordering::Release);
    send_error(error);
    send_state(State::Ready);
}

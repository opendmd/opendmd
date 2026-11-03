//! Minimal captive-portal DNS server.
//!
//! Lifted verbatim from `~/projects/stripe-dashboard/src/dns_server.rs`, which
//! is where the awkward details were worked out. It runs a UDP socket on port
//! 53 and answers *every* query with the AP's own IP, which is what makes the
//! setup page pop up as a "sign in to network" sheet on phones and laptops.
//!
//! Two things in here are not obvious and both are load-bearing:
//!
//! - lwip's socket FFI takes `usize`/`isize` for buffer sizes and return
//!   values on the ESP32, not the POSIX `u32`/`i32`.
//! - the answer is appended after the *question section*, found by walking the
//!   name labels — not at `query_len`, which would corrupt the reply whenever
//!   the query carries an EDNS0 OPT record in its additional section.
//!
//! - only an A (or ANY) question gets the address. Any other type (AAAA, and
//!   the HTTPS/SVCB lookups iOS sends alongside) gets an empty NOERROR: an A
//!   record in reply to one is malformed, and an iPhone that sees it when it
//!   joins skips the "sign in to network" sheet.
//!
//! The AP's DHCP server also has to advertise this address as the DNS server
//! (see `wifi::WifiManager::start_ap`), or iOS and Android use cellular DNS and
//! never see the portal at all.

use log::{info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const DNS_HEADER_SIZE: usize = 12;
const DNS_MAX_PACKET: usize = 512;
const DNS_FLAGS_RESPONSE: u16 = 0x8580; // QR=1, AA=1, RA=1

pub struct DnsServer {
    stop_flag: Arc<AtomicBool>,
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl DnsServer {
    /// Start the DNS server on port 53, responding to all queries with `ip`.
    pub fn start(ip: [u8; 4]) -> anyhow::Result<Self> {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_clone = stop_flag.clone();

        // On CPU0, as every task but the display's (it had no affinity).
        let handle = crate::task::spawn_core0_joinable(
            b"dns-server\0",
            4096,
            5,
            Box::new(move || {
                if let Err(e) = dns_server_loop(ip, &stop_clone) {
                    warn!("DNS server error: {}", e);
                }
                info!("DNS server stopped");
            }),
        )?;

        info!(
            "Captive portal DNS started for {}.{}.{}.{}",
            ip[0], ip[1], ip[2], ip[3]
        );

        Ok(Self {
            stop_flag,
            thread_handle: Some(handle),
        })
    }

    fn shutdown(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for DnsServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Find the byte offset immediately after the first question in a DNS packet.
/// Returns None if the packet is malformed.
fn find_question_end(buf: &[u8]) -> Option<usize> {
    let mut pos = DNS_HEADER_SIZE;
    // Skip name labels
    while pos < buf.len() {
        let len = buf[pos] as usize;
        if len == 0 {
            pos += 1; // null terminator
            break;
        }
        if len & 0xC0 == 0xC0 {
            pos += 2; // compression pointer
            break;
        }
        pos += 1 + len;
    }
    // Skip QTYPE (2) + QCLASS (2)
    pos += 4;
    if pos <= buf.len() {
        Some(pos)
    } else {
        None
    }
}

fn dns_server_loop(ip: [u8; 4], stop_flag: &AtomicBool) -> anyhow::Result<()> {
    unsafe {
        let sock = esp_idf_sys::lwip_socket(
            esp_idf_sys::AF_INET as i32,
            esp_idf_sys::SOCK_DGRAM as i32,
            esp_idf_sys::IPPROTO_UDP as i32,
        );
        if sock < 0 {
            anyhow::bail!("Failed to create DNS socket");
        }

        // 1-second receive timeout so we can check the stop flag
        let timeout = esp_idf_sys::timeval {
            tv_sec: 1,
            tv_usec: 0,
        };
        esp_idf_sys::lwip_setsockopt(
            sock,
            esp_idf_sys::SOL_SOCKET as i32,
            esp_idf_sys::SO_RCVTIMEO as i32,
            &timeout as *const _ as *const core::ffi::c_void,
            core::mem::size_of::<esp_idf_sys::timeval>() as u32,
        );

        // Bind to 0.0.0.0:53
        let mut bind_addr: esp_idf_sys::sockaddr_in = core::mem::zeroed();
        bind_addr.sin_len = core::mem::size_of::<esp_idf_sys::sockaddr_in>() as u8;
        bind_addr.sin_family = esp_idf_sys::AF_INET as u8;
        bind_addr.sin_port = 53u16.to_be();
        bind_addr.sin_addr.s_addr = 0; // INADDR_ANY

        let ret = esp_idf_sys::lwip_bind(
            sock,
            &bind_addr as *const _ as *const esp_idf_sys::sockaddr,
            core::mem::size_of::<esp_idf_sys::sockaddr_in>() as u32,
        );
        if ret < 0 {
            esp_idf_sys::lwip_close(sock);
            anyhow::bail!("Failed to bind DNS socket to port 53");
        }

        let mut buf = [0u8; DNS_MAX_PACKET];
        let mut client_addr: esp_idf_sys::sockaddr_in = core::mem::zeroed();
        let mut addr_len: u32 = core::mem::size_of::<esp_idf_sys::sockaddr_in>() as u32;

        while !stop_flag.load(Ordering::Relaxed) {
            let n = esp_idf_sys::lwip_recvfrom(
                sock,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                DNS_MAX_PACKET,
                0,
                &mut client_addr as *mut _ as *mut esp_idf_sys::sockaddr,
                &mut addr_len,
            );

            if n < DNS_HEADER_SIZE as isize {
                continue; // timeout or runt packet
            }
            let query_len = n as usize;

            // Find where the question section ends
            let question_end = match find_question_end(&buf[..query_len]) {
                Some(end) => end,
                None => continue,
            };

            // Build response: header + question + one A-record answer, or no
            // answer for any other type (see the module comment).
            let qtype = u16::from_be_bytes([buf[question_end - 4], buf[question_end - 3]]);
            let answer = qtype == 1 || qtype == 255;
            let resp_len = question_end + if answer { 16 } else { 0 };
            if resp_len > DNS_MAX_PACKET {
                continue;
            }

            let mut resp = [0u8; DNS_MAX_PACKET];
            resp[..question_end].copy_from_slice(&buf[..question_end]);

            // Response flags
            resp[2] = (DNS_FLAGS_RESPONSE >> 8) as u8;
            resp[3] = (DNS_FLAGS_RESPONSE & 0xFF) as u8;
            // ANCOUNT = 1, or 0
            resp[6] = 0;
            resp[7] = answer as u8;
            // NSCOUNT = 0, ARCOUNT = 0 (clear any additional records from query)
            resp[8] = 0;
            resp[9] = 0;
            resp[10] = 0;
            resp[11] = 0;

            // Answer section: a pointer to the question's name, type A,
            // class IN, TTL 60 s, RDLENGTH 4, the address.
            if answer {
                resp[question_end..resp_len].copy_from_slice(&[
                    0xC0, 0x0C, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3C, 0x00, 0x04, ip[0], ip[1], ip[2], ip[3],
                ]);
            }

            esp_idf_sys::lwip_sendto(
                sock,
                resp.as_ptr() as *const core::ffi::c_void,
                resp_len,
                0,
                &client_addr as *const _ as *const esp_idf_sys::sockaddr,
                addr_len,
            );
        }

        esp_idf_sys::lwip_close(sock);
    }

    Ok(())
}

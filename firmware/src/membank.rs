//! Where the display's hot buffers are (display-isolation, 2026-11-02): the
//! upload residual comes and goes between runs, and one explanation is CPU1
//! and CPU0's DMA traffic (Wi-Fi, the card's SPI, lwIP's buffers) meeting
//! in one internal SRAM pool, which depends on the heap at a Formula's open.
//! `/api/status` `display.buffers` names each buffer's address and pool,
//! the heartbeat logs them, and `/api/status?pools=1` walks the heap for
//! the bytes in use in each pool.
//!
//! The classic's pools are IDF's (`heap/port/esp32/memory_layout.c`): SRAM2
//! at 0x3FFAE000..0x3FFE0000 (pools 16, 15, 14, then 10-13 as 32 KB blocks),
//! SRAM1 at 0x3FFE0000..0x40000000 (pools 9, 8, 7, 6), SRAM0 the IRAM.
//!
//! The ESP32-S3's (`heap/port/esp32s3/memory_layout.c`, the S3 TRM's map)
//! are named by address instead, in 32 KB blocks: SRAM1, the D/IRAM, at
//! 0x3FC88000..0x3FCF0000 (`SRAM1/0`..`SRAM1/12`; its IRAM alias
//! 0x40378000.. runs the same way, 0x6F0000 above), SRAM2 (data only) at
//! 0x3FCF0000..0x3FD00000 (`SRAM2/0`, `SRAM2/1`), SRAM0 (IRAM only) at
//! 0x40370000..0x40378000, RTC fast memory, and `PSRAM` (the octal PSRAM's
//! data mapping, 0x3C000000..0x3E000000, shared with the flash's rodata).

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};

/// SRAM1's IRAM alias (0x400A0000..0x400C0000) as its DRAM address: the
/// two run in opposite directions, word for word.
#[cfg(not(esp32s3))]
fn dram_of(a: usize) -> usize {
    if (0x400A_0000..0x400C_0000).contains(&a) {
        0x3FFE_0000 + (0x400B_FFFC - (a & !3)) + (a & 3)
    } else {
        a
    }
}

/// The S3's SRAM1 IRAM alias (0x40378000..0x403E0000) as its DRAM address:
/// the same direction, 0x6F0000 apart.
#[cfg(esp32s3)]
fn dram_of(a: usize) -> usize {
    if (0x4037_8000..0x403E_0000).contains(&a) {
        a - 0x6F_0000
    } else {
        a
    }
}

/// `"SRAM2/15"`, `"SRAM2/10-13.2"` (the third 32 KB block of 10-13),
/// `"SRAM1/7"`, `"SRAM0"`, or `"?"`.
#[cfg(not(esp32s3))]
pub fn pool(a: usize) -> String {
    let d = dram_of(a);
    match d {
        0x3FFA_E000..=0x3FFA_FFFF => "SRAM2/16".into(),
        0x3FFB_0000..=0x3FFB_7FFF => "SRAM2/15".into(),
        0x3FFB_8000..=0x3FFB_FFFF => "SRAM2/14".into(),
        0x3FFC_0000..=0x3FFD_FFFF => format!("SRAM2/10-13.{}", (d - 0x3FFC_0000) >> 15),
        0x3FFE_0000..=0x3FFE_7FFF => "SRAM1/9".into(),
        0x3FFE_8000..=0x3FFE_FFFF => "SRAM1/8".into(),
        0x3FFF_0000..=0x3FFF_7FFF => "SRAM1/7".into(),
        0x3FFF_8000..=0x3FFF_FFFF => "SRAM1/6".into(),
        0x4007_0000..=0x4009_FFFF => "SRAM0".into(),
        _ => "?".into(),
    }
}

/// The S3's: `"SRAM1/3"` (the fourth 32 KB block of SRAM1), `"SRAM2/1"`,
/// `"SRAM0"`, `"RTC"`, `"PSRAM"`, or `"?"`.
#[cfg(esp32s3)]
pub fn pool(a: usize) -> String {
    let d = dram_of(a);
    match d {
        0x3FC8_8000..=0x3FCE_FFFF => format!("SRAM1/{}", (d - 0x3FC8_8000) >> 15),
        0x3FCF_0000..=0x3FCF_FFFF => format!("SRAM2/{}", (d - 0x3FCF_0000) >> 15),
        0x4037_0000..=0x4037_7FFF => "SRAM0".into(),
        0x600F_E000..=0x600F_FFFF => "RTC".into(),
        0x3C00_0000..=0x3DFF_FFFF => "PSRAM".into(),
        _ => "?".into(),
    }
}

/// The pool keys `pools_json` reports, in address order.
#[cfg(not(esp32s3))]
const POOLS: [(usize, usize, &str); 11] = [
    (0x3FFA_E000, 0x3FFB_0000, "SRAM2/16"),
    (0x3FFB_0000, 0x3FFB_8000, "SRAM2/15"),
    (0x3FFB_8000, 0x3FFC_0000, "SRAM2/14"),
    (0x3FFC_0000, 0x3FFC_8000, "SRAM2/10-13.0"),
    (0x3FFC_8000, 0x3FFD_0000, "SRAM2/10-13.1"),
    (0x3FFD_0000, 0x3FFD_8000, "SRAM2/10-13.2"),
    (0x3FFD_8000, 0x3FFE_0000, "SRAM2/10-13.3"),
    (0x3FFE_0000, 0x3FFE_8000, "SRAM1/9"),
    (0x3FFE_8000, 0x3FFF_0000, "SRAM1/8"),
    (0x3FFF_0000, 0x3FFF_8000, "SRAM1/7"),
    (0x3FFF_8000, 0x4000_0000, "SRAM1/6"),
];

/// The S3's (the heap's part of each: SRAM1's blocks below the heap's start
/// hold the image's IRAM code and static data, and report 0).
#[cfg(esp32s3)]
const POOLS: [(usize, usize, &str); 18] = [
    (0x3FC8_8000, 0x3FC9_0000, "SRAM1/0"),
    (0x3FC9_0000, 0x3FC9_8000, "SRAM1/1"),
    (0x3FC9_8000, 0x3FCA_0000, "SRAM1/2"),
    (0x3FCA_0000, 0x3FCA_8000, "SRAM1/3"),
    (0x3FCA_8000, 0x3FCB_0000, "SRAM1/4"),
    (0x3FCB_0000, 0x3FCB_8000, "SRAM1/5"),
    (0x3FCB_8000, 0x3FCC_0000, "SRAM1/6"),
    (0x3FCC_0000, 0x3FCC_8000, "SRAM1/7"),
    (0x3FCC_8000, 0x3FCD_0000, "SRAM1/8"),
    (0x3FCD_0000, 0x3FCD_8000, "SRAM1/9"),
    (0x3FCD_8000, 0x3FCE_0000, "SRAM1/10"),
    (0x3FCE_0000, 0x3FCE_8000, "SRAM1/11"),
    (0x3FCE_8000, 0x3FCF_0000, "SRAM1/12"),
    (0x3FCF_0000, 0x3FCF_8000, "SRAM2/0"),
    (0x3FCF_8000, 0x3FD0_0000, "SRAM2/1"),
    (0x4037_0000, 0x4037_8000, "SRAM0"),
    (0x600F_E000, 0x6010_0000, "RTC"),
    (0x3C00_0000, 0x3E00_0000, "PSRAM"),
];

/// The canvas's pixels (set once at boot).
pub static CANVAS: AtomicUsize = AtomicUsize::new(0);

/// The player's card reads (its file's `read`/`seek`, `frametime`): the
/// longest single one since the last [`take_read_max`], µs.
pub static READ_MAX_US: AtomicU32 = AtomicU32::new(0);
/// Likewise its opens.
pub static OPEN_MAX_US: AtomicU32 = AtomicU32::new(0);

/// `{"read_max_us":N,"open_max_us":N}`, both reset (`/api/status?take_read_max=1`).
pub fn take_read_max() -> String {
    format!(
        r#"{{"read_max_us":{},"open_max_us":{}}}"#,
        READ_MAX_US.swap(0, Relaxed),
        OPEN_MAX_US.swap(0, Relaxed)
    )
}

fn entry(name: &str, addr: usize, bytes: usize) -> String {
    if addr == 0 {
        return format!(r#""{name}":null"#);
    }
    format!(r#""{name}":{{"at":"{addr:#010x}","bytes":{bytes},"pool":"{}"}}"#, pool(addr))
}

/// `display.buffers`: the playing Formula's memory, the canvas, the panel's
/// DMA rows and descriptors, and the arena (`formula_arena`) if set aside.
pub fn buffers_json() -> String {
    let mut parts = Vec::with_capacity(8);
    #[cfg(feature = "psi")]
    {
        use opendmd_core::fx_psi as fx;
        let b = |k: usize| fx::BUFFERS[k].load(Relaxed);
        parts.push(entry("formula", b(fx::BUF_FX), fx::FX_BYTES));
        parts.push(entry("ink", b(fx::BUF_INK), fx::INK_BYTES));
        parts.push(entry("plane", b(fx::BUF_PLANE), fx::PLANE_BYTES));
        let (_, _, code) = fx::jit_status();
        parts.push(entry("kernel", b(fx::BUF_KERNEL), code as usize));
        let (at, bytes) = crate::arena::span();
        parts.push(entry("arena", at, bytes));
    }
    parts.push(entry("canvas", CANVAS.load(Relaxed), opendmd_core::canvas::PIXELS * 3));
    let (lo, hi, bytes, _n, dlo, dhi) = crate::panel::dma_span();
    let rows = if lo == 0 {
        r#""dma_rows":null"#.to_string()
    } else {
        format!(
            r#""dma_rows":{{"at":"{lo:#010x}","end":"{hi:#010x}","bytes":{bytes},"pool":"{}","pool_end":"{}"}}"#,
            pool(lo as usize),
            pool(hi.saturating_sub(1) as usize)
        )
    };
    parts.push(rows);
    parts.push(entry("dma_desc", dlo as usize, dhi.saturating_sub(dlo) as usize));
    format!("{{{}}}", parts.join(","))
}

/// The heartbeat's line: each buffer's pool, short.
pub fn buffers_line() -> String {
    let mut s = String::new();
    #[cfg(feature = "psi")]
    {
        use opendmd_core::fx_psi as fx;
        for (name, k) in [("formula", fx::BUF_FX), ("ink", fx::BUF_INK), ("plane", fx::BUF_PLANE), ("kernel", fx::BUF_KERNEL)] {
            let a = fx::BUFFERS[k].load(Relaxed);
            if a != 0 {
                s.push_str(&format!(" {name}@{a:#x}={}", pool(a)));
            }
        }
    }
    let (lo, hi, ..) = crate::panel::dma_span();
    if lo != 0 {
        s.push_str(&format!(" dma@{lo:#x}..{hi:#x}={}..{}", pool(lo as usize), pool(hi.saturating_sub(1) as usize)));
    }
    s
}

/// `{"pools":{"SRAM2/15":{"used":N,"blocks":N,"free":N},...}}`: one walk of
/// every heap (CPU0, on request: the display core allocates nothing per
/// frame, so the walk's heap locks never meet it).
pub fn pools_json() -> String {
    #[derive(Default, Clone, Copy)]
    struct Acc {
        used: usize,
        free: usize,
        blocks: u32,
    }
    let mut acc = [Acc::default(); POOLS.len()];
    unsafe extern "C" fn walk(
        _heap: esp_idf_sys::walker_heap_into_t,
        b: esp_idf_sys::walker_block_info_t,
        user: *mut core::ffi::c_void,
    ) -> bool {
        // SAFETY: `user` is `acc`, alive for the walk.
        let acc = unsafe { &mut *(user as *mut [Acc; POOLS.len()]) };
        let a = dram_of(b.ptr as usize);
        let end = a + b.size;
        for (k, &(lo, hi, _)) in POOLS.iter().enumerate() {
            let n = end.min(hi).saturating_sub(a.max(lo));
            if n > 0 {
                if b.used {
                    acc[k].used += n;
                    acc[k].blocks += 1;
                } else {
                    acc[k].free += n;
                }
            }
        }
        true
    }
    // SAFETY: the callback only writes `acc`, passed as its user data.
    unsafe { esp_idf_sys::heap_caps_walk_all(Some(walk), (&mut acc as *mut [Acc; POOLS.len()]).cast()) };
    let body: Vec<String> = POOLS
        .iter()
        .zip(acc.iter())
        .map(|(&(_, _, name), a)| format!(r#""{name}":{{"used":{},"blocks":{},"free":{}}}"#, a.used, a.blocks, a.free))
        .collect();
    format!(r#"{{"pools":{{{}}}}}"#, body.join(","))
}

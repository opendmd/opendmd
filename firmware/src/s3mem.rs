//! The ESP32-S3's memory, reported (S3 port phase 1: a bare DevKitC-1 on
//! USB, measured over HTTP and the console). `GET /api/debug/mem`, and a
//! `s3mem:` console line every [`LINE_EVERY`] from the network worker.
//!
//! Internal RAM (`MALLOC_CAP_INTERNAL`) and the PSRAM pool are reported
//! apart: `esp_get_free_heap_size` and the `MALLOC_CAP_8BIT` /
//! `MALLOC_CAP_DEFAULT` queries take in the PSRAM's 8 MB on the S3, so none
//! of them is read here. Internal RAM is split by what it can hold
//! (`exec`: D/IRAM, which the JIT's kernels share with `malloc`; `dma`; `rtc`,
//! RTC fast memory), each heap region is listed with its pool
//! (`membank::pool`), and the build's placement switches are stated from its
//! sdkconfig (`facts`) as the image's name and switches asked for them
//! (`OPENDMD_S3_IMAGE`, `OPENDMD_S3_SWITCHES`, the Makefile's
//! `fw-s3-image`). A preview build's alone: the classic has no such route.

use crate::httpd::{Method, Respond, Server};
use esp_idf_sys as sys;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

/// The image's name and switches (`make fw-s3-image S3_IMAGE=<name>`).
const IMAGE: &str = match option_env!("OPENDMD_S3_IMAGE") {
    Some(v) => v,
    None => "",
};
const SWITCHES: &str = match option_env!("OPENDMD_S3_SWITCHES") {
    Some(v) => v,
    None => "",
};

/// How often the console line comes (ms).
const LINE_EVERY: u32 = 30_000;

/// (free, largest, lowest since boot, total) for `caps`.
fn pool(caps: u32) -> (usize, usize, usize, usize) {
    // SAFETY: read-only queries with a capability mask.
    unsafe {
        (
            sys::heap_caps_get_free_size(caps),
            sys::heap_caps_get_largest_free_block(caps),
            sys::heap_caps_get_minimum_free_size(caps),
            sys::heap_caps_get_total_size(caps),
        )
    }
}

fn pool_json(caps: u32) -> String {
    let (free, largest, min, total) = pool(caps);
    format!(r#"{{"free":{free},"largest":{largest},"min":{min},"total":{total}}}"#)
}

const INTERNAL: u32 = sys::MALLOC_CAP_INTERNAL | sys::MALLOC_CAP_8BIT;

/// Each heap region: its start, end, pool, bytes used and free, blocks used.
fn regions_json() -> String {
    #[derive(Clone, Copy, Default)]
    struct R {
        start: usize,
        end: usize,
        used: usize,
        free: usize,
        blocks: u32,
    }
    const MAX: usize = 16;
    let mut acc = ([R::default(); MAX], 0usize);
    unsafe extern "C" fn walk(
        heap: sys::walker_heap_into_t,
        b: sys::walker_block_info_t,
        user: *mut core::ffi::c_void,
    ) -> bool {
        // SAFETY: `user` is `acc`, alive for the walk.
        let (rs, n) = unsafe { &mut *(user as *mut ([R; MAX], usize)) };
        let (start, end) = (heap.start as usize, heap.end as usize);
        let k = match rs[..*n].iter().position(|r| r.start == start) {
            Some(k) => k,
            None if *n < MAX => {
                rs[*n] = R { start, end, ..R::default() };
                *n += 1;
                *n - 1
            }
            None => return true,
        };
        if b.used {
            rs[k].used += b.size;
            rs[k].blocks += 1;
        } else {
            rs[k].free += b.size;
        }
        true
    }
    // SAFETY: the callback only writes `acc`, passed as its user data.
    unsafe { sys::heap_caps_walk_all(Some(walk), (&mut acc as *mut ([R; MAX], usize)).cast()) };
    let (rs, n) = acc;
    let body: Vec<String> = rs[..n]
        .iter()
        .map(|r| {
            format!(
                r#"{{"start":"{:#010x}","end":"{:#010x}","bytes":{},"pool":"{}","used":{},"free":{},"blocks":{}}}"#,
                r.start,
                r.end,
                r.end - r.start,
                crate::membank::pool(r.start),
                r.used,
                r.free,
                r.blocks
            )
        })
        .collect();
    format!("[{}]", body.join(","))
}

/// The build's placement facts, from its sdkconfig (esp-idf-sys's
/// `esp_idf_*` cfgs for the booleans, its constants for the numbers).
#[allow(unexpected_cfgs)]
fn facts_json() -> String {
    let b = |v: bool| if v { "true" } else { "false" };
    format!(
        concat!(
            r#"{{"spiram":{},"spiram_use_caps_alloc":{},"spiram_use_malloc":{},"spiram_try_allocate_wifi_lwip":{},"#,
            r#""spiram_allow_stack_external_memory":{},"fatfs_alloc_prefer_extram":{},"#,
            r#""mbedtls_internal_mem_alloc":{},"mbedtls_external_mem_alloc":{},"mbedtls_dynamic_buffer":{},"#,
            r#""wifi_static_rx":{},"wifi_dynamic_rx":{},"wifi_dynamic_tx":{},"wifi_cache_tx":{},"wifi_tx_buffer_type":{},"#,
            r#""lwip_tcp_wnd":{},"lwip_tcp_snd_buf":{},"lwip_max_sockets":{},"#,
            r#""iram":{{"wifi_iram_opt":{},"wifi_rx_iram_opt":{},"wifi_extra_iram_opt":{},"wifi_slp_iram_opt":{},"lwip_iram_optimization":{},"#,
            r#""freertos_in_flash":{},"ringbuf_in_flash":{},"heap_in_flash":{},"spi_flash_rom_impl":{},"spi_master_isr_in_iram":{},"memprot":{}}}}}"#
        ),
        b(cfg!(esp_idf_spiram)),
        b(cfg!(esp_idf_spiram_use_caps_alloc)),
        b(cfg!(esp_idf_spiram_use_malloc)),
        b(cfg!(esp_idf_spiram_try_allocate_wifi_lwip)),
        b(cfg!(esp_idf_spiram_allow_stack_external_memory)),
        b(cfg!(esp_idf_fatfs_alloc_prefer_extram)),
        b(cfg!(esp_idf_mbedtls_internal_mem_alloc)),
        b(cfg!(esp_idf_mbedtls_external_mem_alloc)),
        b(cfg!(esp_idf_mbedtls_dynamic_buffer)),
        sys::CONFIG_ESP_WIFI_STATIC_RX_BUFFER_NUM,
        sys::CONFIG_ESP_WIFI_DYNAMIC_RX_BUFFER_NUM,
        sys::CONFIG_ESP_WIFI_DYNAMIC_TX_BUFFER_NUM,
        sys::CONFIG_ESP_WIFI_CACHE_TX_BUFFER_NUM,
        sys::CONFIG_ESP_WIFI_TX_BUFFER_TYPE,
        sys::CONFIG_LWIP_TCP_WND_DEFAULT,
        sys::CONFIG_LWIP_TCP_SND_BUF_DEFAULT,
        sys::CONFIG_LWIP_MAX_SOCKETS,
        b(cfg!(esp_idf_esp_wifi_iram_opt)),
        b(cfg!(esp_idf_esp_wifi_rx_iram_opt)),
        b(cfg!(esp_idf_esp_wifi_extra_iram_opt)),
        b(cfg!(esp_idf_esp_wifi_slp_iram_opt)),
        b(cfg!(esp_idf_lwip_iram_optimization)),
        b(cfg!(esp_idf_freertos_place_functions_into_flash)),
        b(cfg!(esp_idf_ringbuf_place_functions_into_flash)),
        b(cfg!(esp_idf_heap_place_function_into_flash)),
        b(cfg!(esp_idf_spi_flash_rom_impl)),
        b(cfg!(esp_idf_spi_master_isr_in_iram)),
        b(cfg!(esp_idf_esp_system_memprot_feature)),
    )
}

/// The cold buffers' counts (`psram-cold`), or null.
fn cold_json() -> String {
    #[cfg(feature = "psram-cold")]
    return crate::arena::cold::json();
    #[cfg(not(feature = "psram-cold"))]
    "null".into()
}

fn uptime_s() -> u64 {
    // SAFETY: a plain clock read.
    (unsafe { sys::esp_timer_get_time() } / 1_000_000) as u64
}

/// `GET /api/debug/mem`'s answer.
pub fn json() -> String {
    format!(
        concat!(
            r#"{{"image":"{}","switches":"{}","uptime_s":{},"#,
            r#""internal":{},"exec":{},"dma":{},"rtc":{},"psram":{},"#,
            r#""status_floor":{},"regions":{},"cold":{},"facts":{}}}"#
        ),
        IMAGE,
        SWITCHES,
        uptime_s(),
        pool_json(INTERNAL),
        pool_json(sys::MALLOC_CAP_EXEC),
        pool_json(sys::MALLOC_CAP_DMA | sys::MALLOC_CAP_INTERNAL),
        pool_json(sys::MALLOC_CAP_RTCRAM),
        pool_json(sys::MALLOC_CAP_SPIRAM),
        crate::state::min_free_heap(),
        regions_json(),
        cold_json(),
        facts_json(),
    )
}

/// The console line: internal free / largest / lowest, exec free, PSRAM
/// free / largest.
pub fn line() -> String {
    let (f, l, m, _) = pool(INTERNAL);
    let (xf, xl, ..) = pool(sys::MALLOC_CAP_EXEC);
    let (pf, pl, pm, _) = pool(sys::MALLOC_CAP_SPIRAM);
    format!(
        "s3mem: {} up {} s: internal {f} B free, largest {l}, lowest {m}; exec {xf} (largest {xl}); psram {pf} free, largest {pl}, lowest {pm}",
        if IMAGE.is_empty() { "s3" } else { IMAGE },
        uptime_s()
    )
}

/// From the network worker's loop (CPU0): the console line, every
/// [`LINE_EVERY`].
pub fn tick() {
    static LAST: AtomicU32 = AtomicU32::new(0);
    // SAFETY: a plain clock read.
    let now = (unsafe { sys::esp_timer_get_time() } / 1000) as u32;
    let last = LAST.load(Relaxed);
    if last != 0 && now.wrapping_sub(last) < LINE_EVERY {
        return;
    }
    LAST.store(now.max(1), Relaxed);
    log::info!("{}", line());
    log::info!("{}", crate::s3panel::line());
    // Once, the first time: what the image is and what its sdkconfig says.
    if last == 0 {
        log::info!(r#"s3mem: image "{IMAGE}" ({SWITCHES}), facts {}"#, facts_json());
    }
}

/// `GET /api/debug/mem`, registered before the catch-all (`http::start`).
pub fn routes(server: &mut Server) -> anyhow::Result<()> {
    server.route("/api/debug/mem", Method::Get, |req| req.json(&json()))?;
    crate::s3panel::routes(server)?;
    Ok(())
}

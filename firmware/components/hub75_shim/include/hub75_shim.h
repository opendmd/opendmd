/*
 * hub75_shim - a pure C facade over mrcodetastic/ESP32-HUB75-MatrixPanel-I2S-DMA.
 *
 * This header is parsed by bindgen (via esp-idf-sys's extra_components
 * mechanism, see firmware/Cargo.toml), so it must stay valid C: no classes,
 * no references, no default arguments. The C++ side lives in hub75_shim.cpp.
 */
#ifndef HUB75_SHIM_H
#define HUB75_SHIM_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Mirrors HUB75_I2S_CFG::shift_driver. */
#define HUB75_DRIVER_SHIFTREG 0
#define HUB75_DRIVER_FM6124   1
#define HUB75_DRIVER_FM6126A  2
#define HUB75_DRIVER_ICN2038S 3
#define HUB75_DRIVER_MBI5124  4
#define HUB75_DRIVER_DP3246   5

/* Mirrors HUB75_I2S_CFG::clk_speed (the enum values are the literal Hz). */
#define HUB75_SPEED_8M  8000000
#define HUB75_SPEED_16M 16000000
#define HUB75_SPEED_20M 20000000

/*
 * Panel geometry, pin map and timing. Pins are GPIO numbers; -1 means unused
 * (E is unused at 1/16 scan on the DMDos board -- see hardware/PINOUT.md).
 */
typedef struct {
    /* Geometry: one module is mx_width x mx_height, chain_length of them. */
    uint16_t mx_width;
    uint16_t mx_height;
    uint16_t chain_length;

    /* GPIO map, in the order HUB75_I2S_CFG::i2s_pins declares them. */
    int8_t r1;
    int8_t g1;
    int8_t b1;
    int8_t r2;
    int8_t g2;
    int8_t b2;
    int8_t a;
    int8_t b;
    int8_t c;
    int8_t d;
    int8_t e;
    int8_t lat;
    int8_t oe;
    int8_t clk;

    /* One of HUB75_DRIVER_*. */
    int32_t driver;
    /* DMA double buffering; costs a second framebuffer in DRAM. */
    bool double_buff;
    /* One of HUB75_SPEED_*. */
    int32_t i2sspeed;
    /* Clock cycles to blank OE either side of LAT. */
    uint8_t latch_blanking;
    /* false: data clocked on the negative edge, true: positive edge. */
    bool clkphase;
    /* Minimum scan rate in Hz; drives the LSBMSB transition bit. */
    uint8_t min_refresh_rate;
} hub75_cfg_t;

/*
 * Allocate the DMA buffers and start the I2S engine. Returns false if the
 * panel could not be started (almost always a DMA allocation failure), in
 * which case the library also logs "!KABOOM! ...".
 * Safe to call once; a second call is a no-op returning true.
 */
bool hub75_init(const hub75_cfg_t *cfg);

/*
 * Turn the panel off, from any task on either core, before or after
 * hub75_init: OE (active low) is taken off the I2S matrix and driven high as
 * a plain GPIO output, then (once the panel is up) the I2S DMA is stopped.
 * For a restart: esp_restart on the ESP32 resets only the CPUs, not I2S nor
 * the GPIO matrix, so without this the I2S DMA goes on clocking out whatever
 * the bootloader and the next app's start-up leave in its buffers, with OE
 * wherever those bits say. OE driven high stays high through that reset
 * (GPIO_OUT and the matrix are not reset) until the next hub75_init routes it
 * back to I2S. Also registered with esp_register_shutdown_handler by
 * hub75_init, so any esp_restart blanks the panel; the resets that skip
 * esp_restart's handlers (a panic, a watchdog) are covered at the next boot
 * by an early start-up hook (hub75_shim.cpp). No heap; idempotent.
 */
void hub75_blank(int8_t oe_gpio);

/* 0..255. No-op before hub75_init. */
void hub75_set_brightness(uint8_t brightness);

/*
 * The two timing values that change live, with no reallocation (the panel
 * settings' trial, docs/firmware.md "Panel timing"). No-ops before
 * hub75_init.
 *
 * hub75_set_clk_phase: the CLK pin's inversion in the GPIO matrix -- the
 * only thing clkphase ever sets (Bus_Parallel16::init). Ignored for the
 * MBI5124 and DP3246 drivers, which the library always clocks on the
 * rising edge.
 *
 * hub75_set_latch_blanking: 1..4 clocks; the OE bits re-rendered at the
 * current brightness (setBrightnessOE rewrites every column's OE bit, so
 * the buffer ends as a fresh begin() with that blanking would leave it).
 */
void hub75_set_clk_phase(bool clkphase);
void hub75_set_latch_blanking(uint8_t latch_blanking);

/* Blank the back buffer. */
void hub75_clear(void);

/*
 * Draw a tight RGB888 image into the back buffer, top-left origin, row-major,
 * exactly w * h * 3 bytes. Pixels outside the panel are dropped.
 */
void hub75_draw_rgb888(const uint8_t *rgb, uint16_t w, uint16_t h);

/*
 * One pixel into the back buffer through the library's drawPixelRGB888
 * (~3.3 us each). Out-of-range coordinates are dropped.
 */
void hub75_set_pixel(uint16_t x, uint16_t y, uint8_t r, uint8_t g, uint8_t b);

/*
 * Pixels x0..x1 (half-open) of row y into the back buffer. `rgb_row` points
 * at the start of that row of a tight RGB888 image (pixel x is at
 * rgb_row + 3 * x). Writes exactly the bits drawPixelRGB888 would, with the
 * per-row work hoisted out of the pixel loop; clipped to the panel.
 */
void hub75_draw_span(const uint8_t *rgb_row, uint16_t y, uint16_t x0, uint16_t x1);

/*
 * Rows y and y + height/2 (y < height/2), all of both, from two full RGB888
 * rows: the two rows share every DMA word, so this is one read-modify-write
 * per word where two hub75_draw_span calls would be two.
 */
void hub75_draw_row_pair(const uint8_t *top_row, const uint8_t *bottom_row, uint16_t y);

/* FNV-1a of the whole back buffer (every row, every colour plane): a self-test. */
uint32_t hub75_fb_checksum(void);

/* Swap the DMA buffers. No-op when double_buff was false. */
void hub75_flip(void);

/* Achieved scan rate in Hz, saturated to 255. 0 before hub75_init. */
uint8_t hub75_refresh_rate(void);

/*
 * Where the scan is: the row pair the I2S DMA is sending and how far into
 * it, as microseconds of scan time into the refresh (0 .. 16 * row_us).
 * Read from the DMA's current-descriptor register and the descriptor
 * layout (one descriptor of all colour planes, then equal ones for the
 * extra passes of the high planes, per row pair). One register read and
 * a division; any core, any time. Returns UINT32_MAX when it is not known:
 * before hub75_scan_init, after hub75_blank, double-buffered, or a
 * descriptor layout this does not model.
 */
uint32_t hub75_scan_pos(void);

/*
 * psi-probe: the address range the DMA reads the picture from — the lowest
 * and highest byte of the frame buffer's row-pair buffers (*lo, *hi), their
 * bytes in all (*bytes) and their count (*n) — and the descriptor block's
 * range (*dlo, *dhi). All 0 before hub75_init.
 */
void hub75_dma_span(uint32_t *lo, uint32_t *hi, uint32_t *bytes, uint32_t *n, uint32_t *dlo, uint32_t *dhi);

/*
 * Set up hub75_scan_pos (after hub75_init, before the first read): check
 * the descriptor layout and time the refresh (spins ~2-3 refreshes). Until
 * it has, and if it cannot, hub75_scan_pos is unknown.
 */
void hub75_scan_init(void);

/*
 * One row pair's share of a refresh, in microseconds of scan time: the
 * refresh period measured at hub75_init (two wraps of hub75_scan_pos,
 * timed) divided by the row pairs. 0 when hub75_scan_pos is unknown.
 */
uint32_t hub75_scan_row_us(void);

/*
 * The ESP32-S3 only (hub75_shim.cpp's end; not defined for the classic
 * ESP32, never called there).
 *
 * The GDMA register hub75_scan_pos reads for the descriptor being sent:
 * 0 OUT_DSCR (the default), 1 OUT_DSCR_BF0, 2 OUT_DSCR_BF1, 3 OUT_STATE's
 * address. Before hub75_scan_init.
 */
void hub75_scan_set_source(uint8_t source);

/*
 * Have hub75_scan_init sample the address pins (A-D) beside each of those
 * registers and log how often each agrees ("scan: register ... agrees with
 * the address pins k/n"): bench and test builds. Before hub75_scan_init.
 */
void hub75_scan_set_pins_check(bool on);

/*
 * Power-safe (the brownout-loop guard): hub75_init does not start the panel
 * (OE driven high, every other HUB75 pin undriven) and returns true; every
 * other call is then a no-op. Before hub75_init.
 */
void hub75_set_power_safe(bool on);

/* hub75_init's first step on the S3 (hub75_shim_s3.inc); nothing here. */
#define HUB75_S3_INIT_HOOK

/*
 * The ESP32-S3 only, double buffering (hub75_shim.cpp's end; not defined
 * for the classic ESP32, never called there).
 *
 * hub75_set_buffers: 2 runs the driver double-buffered when the internal
 * DMA-capable heap has room for the second frame buffer and descriptor ring
 * (else single, the beam raced, logged why); 1 single; 0 the build's default
 * (double where WiFi's and lwIP's buffers go to PSRAM first, as the carrier
 * image's; else single). Before hub75_init.
 *
 * Double-buffered, every write goes into the back buffer and hub75_flip
 * presents it: the DMA moves onto it at the end of the refresh it is
 * sending. The back buffer is not written again until the DMA is seen in
 * the other ring (GDMA's OUT_DSCR); then the pairs the last frame wrote are
 * copied across, so the back buffer always holds what is shown plus what
 * has been written since (as single-buffered: pushing only what changed
 * still works). hub75_s3_back_ready waits for that (asleep a tick at a time
 * while the DMA is far from the ring's end, then spinning) and returns the
 * us it waited; every writer calls it first anyway.
 */
typedef struct {
    /* 1 or 2: what the panel runs with; and the build's default (0 asked:
     * 2 where WiFi's and lwIP's buffers go to PSRAM first, else 1). */
    uint8_t buffers;
    uint8_t build_buffers;
    /* Why not 2 (HUB75_S3_WHY_*). */
    uint8_t why;
    /* The refresh measured at start-up (us), 0 unknown. */
    uint32_t refresh_us;
    /* Each frame buffer's bytes and each descriptor ring's (2 x each). */
    uint32_t fb_bytes;
    uint32_t desc_bytes;
    /* Internal heap (MALLOC_CAP_INTERNAL) free and largest block before
     * and after the driver started. */
    uint32_t heap_before;
    uint32_t largest_before;
    uint32_t heap_after;
    uint32_t largest_after;
    /* Since boot: flips asked; skipped (nothing written since the last);
     * switches seen (the DMA in the new ring); waits for one before a write,
     * their us in all (wrapping) and the longest; switches whose old ring was
     * still seen more than 1 ms past the end of the refresh the flip was
     * asked in (late_wrap:
     * those asked in the ring's last two descriptors, whose link to the next
     * ring the DMA may already have read: a refresh later, expected; late:
     * the rest, which should not happen); switches never
     * seen (given up after 3 refreshes); writes that found the DMA in the
     * ring being written (the instrumented check: must stay 0); row pairs
     * copied across after a switch. */
    uint32_t flips;
    uint32_t skipped;
    uint32_t switched;
    uint32_t waits;
    uint32_t wait_us;
    uint32_t wait_max_us;
    uint32_t late;
    uint32_t late_wrap;
    uint32_t timeouts;
    uint32_t back_hits;
    uint32_t copied_pairs;
    /* The refreshes each frame was shown, from its switch to the next one's
     * (a switch's time: watched, or when it was due): 1, 2, 3 or more. */
    uint32_t shown_1;
    uint32_t shown_2;
    uint32_t shown_3plus;
} hub75_s3_dbuf_t;

#define HUB75_S3_WHY_OK 0
#define HUB75_S3_WHY_ASKED 1    /* hub75_set_buffers(1) */
#define HUB75_S3_WHY_HEAP 2     /* too little internal DMA heap */
#define HUB75_S3_WHY_BEGIN 3    /* begin() failed double-buffered */
#define HUB75_S3_WHY_RING 4     /* no GDMA channel id, or forced (dbuf_test ring); a second
                                 * ring the library could not allocate never gets here (it
                                 * writes through the null pointer in setupDMA): the heap
                                 * check keeps that from happening */
#define HUB75_S3_WHY_NO_PANEL 5 /* no panel (power-safe, or not started) */
#define HUB75_S3_WHY_BUILD 6    /* the build's default (WiFi's buffers in internal RAM) */

void hub75_set_buffers(uint8_t buffers);
uint32_t hub75_s3_back_ready(void);
void hub75_s3_dbuf(hub75_s3_dbuf_t *out);

/*
 * Test builds (s3panel.rs: bench, nocard-test, or OPENDMD_S3_TEST_HOOKS at
 * the build): hub75_s3_set_test forces a fallback to single buffering
 * before hub75_init (HUB75_S3_TEST_HEAP: the heap check refuses; _RING: the
 * double-buffered start is torn down and started again single-buffered).
 * hub75_s3_fb_sums: the last flip resolved, the FNV-1a of the front and the
 * back frame buffer (false single-buffered); they must agree.
 * hub75_s3_skip_copy: the next resolve skips this pair's copy across (the
 * check's red variant).
 */
#define HUB75_S3_TEST_NONE 0
#define HUB75_S3_TEST_HEAP 1
#define HUB75_S3_TEST_RING 2
void hub75_s3_set_test(uint8_t test);
bool hub75_s3_fb_sums(uint32_t *front, uint32_t *back);
void hub75_s3_skip_copy(uint8_t pair);

/* The writers' and hub75_flip's first step on the S3 (hub75_shim_s3.inc);
 * nothing here. */
#define HUB75_S3_WRITE_HOOK(row)
#define HUB75_S3_FLIP_HOOK

#ifdef __cplusplus
}
#endif

#endif /* HUB75_SHIM_H */

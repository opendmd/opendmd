// C++ side of the pure-C facade declared in include/hub75_shim.h.
//
// Everything C++ about mrcodetastic/ESP32-HUB75-MatrixPanel-I2S-DMA (the
// class, the config struct, its enums) is confined here so that bindgen only
// ever sees the C header.

#include "hub75_shim.h"

#include <string.h>

#include "ESP32-HUB75-MatrixPanel-I2S-DMA.h"
#include "esp_attr.h"
#include "esp_log.h"
#include "esp_private/startup_internal.h"
#include "esp_rom_gpio.h"
#include "esp_system.h"
#include "esp_timer.h"
#include "hal/gpio_ll.h"
#include "soc/gpio_sig_map.h"
#include "soc/soc_caps.h"
#if defined(CONFIG_IDF_TARGET_ESP32S3)
#include "hub75_shim_s3.inc"
#endif

static const char *TAG = "hub75_shim";

static MatrixPanel_I2S_DMA *s_panel = nullptr;
static uint16_t s_width = 0;
static uint16_t s_height = 0;
/* OE's GPIO, for the shutdown handler (-1 before hub75_init). */
static int8_t s_oe = -1;
/* hub75_blank has stopped the DMA (it is not restarted). */
static bool s_stopped = false;
/* The CLK pin and the driver, for hub75_set_clk_phase (-1 before init). */
static int8_t s_clk = -1;
static int32_t s_driver = 0;
/* The last brightness set: hub75_set_latch_blanking re-renders OE at it. */
static uint8_t s_brightness = 128;
/* hub75_draw_span's fast path is usable (see hub75_init). */
static bool s_fast = false;
/*
 * s_spread[v]: the CIE-corrected value lumConvTab[v] with its bit d moved to
 * bit 3*d, for d < PIXEL_COLOR_DEPTH_BITS (<= 8, so 24 bits). A pixel's
 * spread(r) | spread(g) << 1 | spread(b) << 2 then holds, at bits 3d..3d+2,
 * exactly the B/G/R triple updateMatrixDMABuffer() computes for colour plane
 * d, so the per-plane work is one shift and one mask. 1 KB, built at init.
 */
static uint32_t s_spread[256];

/*
 * Access to MatrixPanel_I2S_DMA::fb, the frame buffer the library's own
 * updateMatrixDMABuffer() writes into. It is private and has no accessor, and
 * the library's only per-pixel entry point (drawPixelRGB888 ->
 * updateMatrixDMABuffer) costs ~3.3 us a pixel on this chip: 13.6 ms for a
 * full 128x32 frame. hub75_draw_span writes the same bits a row at a time.
 *
 * This is the standard-conforming "explicit instantiation" loophole, not a
 * `#define private public`: [temp.explicit] says access checking does not
 * apply to the names in an explicit instantiation, so the member pointer can
 * be named there and handed out through a friend function. It reads the
 * member the library already maintains; nothing about the class changes.
 */
template <typename Tag, typename Tag::type M>
struct Rob {
    friend typename Tag::type get(Tag) { return M; }
};
struct FbTag {
    typedef frameStruct *MatrixPanel_I2S_DMA::*type;
    friend type get(FbTag);
};
template struct Rob<FbTag, &MatrixPanel_I2S_DMA::fb>;

/*
 * The same idiom for the DMA descriptor block, for hub75_scan_pos: the
 * panel's Bus_Parallel16 (protected) and its descriptor array and count
 * (private). Read once at init; nothing is written through them.
 */
struct BusTag {
    typedef Bus_Parallel16 MatrixPanel_I2S_DMA::*type;
    friend type get(BusTag);
};
template struct Rob<BusTag, &MatrixPanel_I2S_DMA::dma_bus>;
struct DescTag {
    typedef HUB75_DMA_DESCRIPTOR_T *Bus_Parallel16::*type;
    friend type get(DescTag);
};
template struct Rob<DescTag, &Bus_Parallel16::_dmadesc_a>;
struct DescCountTag {
    typedef uint32_t Bus_Parallel16::*type;
    friend type get(DescCountTag);
};
template struct Rob<DescCountTag, &Bus_Parallel16::_dmadesc_count>;

/*
 * The scan model (hub75_scan_pos): the descriptor block's address range,
 * descriptors per row pair, the first descriptor's bytes (every plane once)
 * and the others' (one plane's extra pass), and the measured scan time of
 * a row pair. s_scan_desc == 0 means unknown.
 */
static uint32_t s_scan_desc = 0;
static uint32_t s_scan_count = 0;
static uint32_t s_scan_per_row = 0;
static uint32_t s_scan_first = 0;
static uint32_t s_scan_rest = 0;
static uint32_t s_scan_row_bytes = 0;
static uint32_t s_scan_row_us = 0;

/*
 * OE driven high as a plain GPIO: GPIO_OUT's bit first, output enable, the
 * pad's matrix output back from I2S to GPIO_OUT, the pad's IO_MUX function
 * last (a no-op once the library has selected it; after a power-on the pad
 * is MTDO). Register writes only, no locks: fine in the early init hook,
 * from a shutdown handler, or on either core.
 */
static bool oe_pin_ok(uint32_t pin) {
    return pin < SOC_GPIO_PIN_COUNT && ((SOC_GPIO_VALID_OUTPUT_GPIO_MASK >> pin) & 1U) != 0;
}

static void oe_high(uint32_t pin) {
    gpio_ll_set_level(&GPIO, pin, 1);
    gpio_ll_output_enable(&GPIO, pin);
    esp_rom_gpio_connect_out_signal(pin, SIG_GPIO_OUT_IDX, false, false);
    esp_rom_gpio_pad_select_gpio(pin);
}

extern "C" void hub75_blank(int8_t oe_gpio) {
    if (oe_gpio >= 0 && oe_pin_ok(oe_gpio)) {
        oe_high(static_cast<uint32_t>(oe_gpio));
    }
    if (s_panel != nullptr && !s_stopped) {
        s_stopped = true;
        // Black buffers (memory only), then the I2S DMA stopped.
        s_panel->stopDMAoutput();
    }
}

static void blank_on_shutdown(void) {
    hub75_blank(s_oe);
}

/*
 * OE's pin, kept for the next boot's early hook: RTC slow memory survives
 * the CPU-only resets (esp_restart, a panic, the watchdogs) that leave I2S1
 * and the GPIO matrix running. The pin in the low byte, a tag above it.
 */
#define OE_RTC_TAG 0x0E0E7500u
RTC_NOINIT_ATTR static uint32_t s_rtc_oe;

/*
 * The earliest the app can blank the panel: IDF's CORE start-up stage,
 * priority 2 (after the eFuse check, before the heap, and before the
 * brownout detector is armed at 104), on CPU0 — about 0.75 s after the
 * reset, the bootloader's time. After a CPU-only reset that did not go
 * through hub75_blank (a panic, a watchdog), the last run's I2S DMA is still
 * clocking its buffers — being overwritten by the bootloader and this
 * start-up — onto the panel, OE included; this stops that at OE. After a
 * power-on, brownout or EN reset the pads were reset (GPIO15 is an input with
 * its pull-up: OE off) and this only drives what the pull-up already does.
 * It is not a strapping concern: MTDO is latched only at a chip reset, and
 * high is its default (ROM boot log on); VDD_SDIO is MTDI (GPIO12, G2).
 */
ESP_SYSTEM_INIT_FN(hub75_early_blank, CORE, BIT(0), 2) {
    const uint32_t v = s_rtc_oe;
    const uint32_t pin = v & 0xFFu;
    if ((v & ~0xFFu) == OE_RTC_TAG && oe_pin_ok(pin)) {
        oe_high(pin);
    }
    return ESP_OK;
}

/*
 * The DMA's current descriptor, as an index into the block (UINT32_MAX
 * outside it). The ESP32's OUTLINK_DSCR register holds the address of the
 * descriptor the DMA is on, and hub75_scan_pos reports that descriptor's
 * start, so a reading is coarse: it lags by up to one descriptor's time —
 * ~102 us inside a pair's first descriptor (2,048 B, every colour plane;
 * on a test panel's measured clock), ~13 us inside the others — and with burst
 * prefetch it may already show the next descriptor, leading by as much.
 * The pacer absorbs this in its per-pair write allowance (300 us, 450 us
 * tinted, against ~74 us measured for a full pair), not in its 30 us
 * margin. The seam count (Judge) shares the error: a pair written within
 * ~0.1 ms of its slot's edges can be counted in_pair/torn when it was not,
 * or the other way, so in_pair and torn are good to about that resolution.
 */
static inline uint32_t scan_desc_index(void) {
#if defined(CONFIG_IDF_TARGET_ESP32S3)
    /* The S3's LCD_CAM is fed by a GDMA channel, whose own registers hold
     * the descriptor it is on: OUT_DSCR by default, or OUT_DSCR_BF0/BF1 or
     * OUT_STATE's address (hub75_scan_set_source); s3_scan_desc_index, at
     * the end of this file. */
    return s3_scan_desc_index();
#else
    const uint32_t addr = getDev()->out_link_dscr;
    const uint32_t off = addr - s_scan_desc;
    const uint32_t idx = off / sizeof(HUB75_DMA_DESCRIPTOR_T);
    return idx < s_scan_count ? idx : UINT32_MAX;
#endif
}

/* In IRAM, as the span writers: read twice a row pair while a Formula
 * races the beam, between renders whose flash code would evict it. */
extern "C" IRAM_ATTR uint32_t hub75_scan_pos(void) {
    if (s_scan_desc == 0 || s_stopped) {
        return UINT32_MAX;
    }
    const uint32_t idx = scan_desc_index();
    if (idx == UINT32_MAX) {
        return UINT32_MAX;
    }
    const uint32_t row = idx / s_scan_per_row;
    const uint32_t j = idx % s_scan_per_row;
    const uint32_t bytes = j == 0 ? 0 : s_scan_first + (j - 1) * s_scan_rest;
    return row * s_scan_row_us + bytes * s_scan_row_us / s_scan_row_bytes;
}

extern "C" uint32_t hub75_scan_row_us(void) {
    return s_scan_desc == 0 ? 0 : s_scan_row_us;
}

/*
 * Check the descriptor layout against the model and time the refresh:
 * the instants of two successive wraps of the descriptor index (~10 ms
 * apart; this spins for up to ~3 refreshes once, at boot).
 */
extern "C" void hub75_scan_init(void) {
    s_scan_desc = 0;
    if (s_panel == nullptr || s_stopped) {
        return;
    }
#if defined(CONFIG_IDF_TARGET_ESP32S3)
    s3_scan_init(); /* (at the end of this file) */
#else
    const uint16_t rows = static_cast<uint16_t>(s_height / MATRIX_ROWS_IN_PARALLEL);
    if (s_panel->getCfg().double_buff || rows == 0) {
        return;
    }
    Bus_Parallel16 &bus = s_panel->*get(BusTag());
    HUB75_DMA_DESCRIPTOR_T *desc = bus.*get(DescTag());
    const uint32_t count = bus.*get(DescCountTag());
    if (desc == nullptr || count == 0 || count % rows != 0) {
        ESP_LOGW(TAG, "scan: layout not modelled");
        return;
    }
    const uint32_t per_row = count / rows;
    const uint32_t first = desc[0].length;
    const uint32_t rest = per_row > 1 ? desc[1].length : 0;
    for (uint32_t i = 0; i < count; i++) {
        const uint32_t want = (i % per_row) == 0 ? first : rest;
        if (desc[i].length != want) {
            ESP_LOGW(TAG, "scan: layout not modelled");
            return;
        }
    }
    s_scan_count = count;
    s_scan_per_row = per_row;
    s_scan_first = first;
    s_scan_rest = rest;
    s_scan_row_bytes = first + (per_row - 1) * rest;
    s_scan_desc = reinterpret_cast<uint32_t>(desc);
    // Time four wraps (the index going back down) of the running DMA and
    // take the shortest of the three intervals (a preempted read only
    // makes one longer). Implausible against the library's own computed
    // rate (outside 0.5-1.5x): the scan is unknown.
    int64_t wraps[4] = {0, 0, 0, 0};
    int seen = 0;
    uint32_t last = scan_desc_index();
    const int64_t give_up = esp_timer_get_time() + 150000;
    while (seen < 4 && esp_timer_get_time() < give_up) {
        const uint32_t idx = scan_desc_index();
        if (idx != UINT32_MAX && last != UINT32_MAX && idx + count / 2 < last) {
            wraps[seen++] = esp_timer_get_time();
        }
        if (idx != UINT32_MAX) {
            last = idx;
        }
    }
    if (seen < 4) {
        ESP_LOGW(TAG, "scan: no refresh in 150 ms");
        s_scan_desc = 0;
        return;
    }
    uint32_t period = UINT32_MAX;
    for (int i = 1; i < 4; i++) {
        const uint32_t d = static_cast<uint32_t>(wraps[i] - wraps[i - 1]);
        period = d < period ? d : period;
    }
    const int calc = s_panel->calculated_refresh_rate;
    const uint32_t expect = calc > 0 ? 1000000u / static_cast<uint32_t>(calc) : 0;
    if (expect == 0 || period * 3 < expect * 2 || period > expect * 2) {
        ESP_LOGW(TAG, "scan: refresh %u us measured, %u us expected: implausible, not used",
                 (unsigned)period, (unsigned)expect);
        s_scan_desc = 0;
        return;
    }
    s_scan_row_us = period / rows;
    ESP_LOGI(TAG, "scan: %u descriptors, %u per row pair, refresh %u us", (unsigned)count,
             (unsigned)per_row, (unsigned)period);
#endif
}

extern "C" bool hub75_init(const hub75_cfg_t *cfg) { HUB75_S3_INIT_HOOK /* (the S3: hub75_shim_s3.inc) */
    if (s_panel != nullptr) {
        return true;
    }
    if (cfg == nullptr) {
        return false;
    }

    HUB75_I2S_CFG::i2s_pins pins = {
        cfg->r1, cfg->g1, cfg->b1, cfg->r2, cfg->g2, cfg->b2, cfg->a,
        cfg->b,  cfg->c,  cfg->d,  cfg->e,  cfg->lat, cfg->oe, cfg->clk,
    };

    HUB75_I2S_CFG mxconfig(
        cfg->mx_width,
        cfg->mx_height,
        cfg->chain_length,
        pins,
        static_cast<HUB75_I2S_CFG::shift_driver>(cfg->driver),
        HUB75_I2S_CFG::TYPE138,
        cfg->double_buff,
        static_cast<HUB75_I2S_CFG::clk_speed>(cfg->i2sspeed),
        cfg->latch_blanking,
        cfg->clkphase,
        cfg->min_refresh_rate);

    s_panel = new MatrixPanel_I2S_DMA(mxconfig);
    if (s_panel == nullptr) {
        return false;
    }
    if (!s_panel->begin()) {
        ESP_LOGE(TAG, "MatrixPanel_I2S_DMA::begin() failed");
        delete s_panel;
        s_panel = nullptr;
        return false;
    }

#if defined(CONFIG_IDF_TARGET_ESP32S3) && !defined(SPIRAM_DMA_BUFFER)
    {
        /* The S3's LCD_CAM pixel clock is 160 MHz / lcd_clkm_div_num
         * (lcd_clk_equ_sysclk = 1). The driver picks one of three dividers
         * from the asked clock (16: 10 MHz up to 10 MHz asked, 10: 16 MHz,
         * 7: 22.9 MHz), so the panel timing's 8 MHz ran at 10 and nothing
         * went lower. Set the divider the asked clock names instead
         * (8 MHz -> 20, 16 -> 10, 20 -> 8), for long wires to a carrier.
         * Written once, right after begin(): the clock generator takes it
         * as it comes; the DMA chain does not depend on it. */
        const int32_t hz = cfg->i2sspeed > 0 ? cfg->i2sspeed : 8000000;
        int32_t div = (160000000 + hz - 1) / hz;
        if (div < 2) div = 2;
        if (div > 255) div = 255;
        LCD_CAM.lcd_clock.lcd_clkm_div_num = static_cast<uint32_t>(div);
        LCD_CAM.lcd_clock.lcd_clkm_div_a = 1;
        LCD_CAM.lcd_clock.lcd_clkm_div_b = 0;
        ESP_LOGI(TAG, "panel clock: divider %d, %d kHz (asked %d kHz)", (int)div,
                 (int)(160000 / div), (int)(hz / 1000));
    }
#endif

    s_width = static_cast<uint16_t>(cfg->mx_width * cfg->chain_length);
    s_height = cfg->mx_height;
    // The span writer hardcodes what updateMatrixDMABuffer() decides at
    // compile time (the CIE LUT for PIXEL_COLOR_DEPTH_BITS, no PSRAM
    // write-back) and one runtime assumption: the configured colour depth is
    // that same PIXEL_COLOR_DEPTH_BITS. Anything else takes the per-pixel path.
#if !defined(NO_CIE1931) && LUT_NATIVE_BIT_DEPTH && !defined(SPIRAM_DMA_BUFFER)
    s_fast = s_panel->getCfg().getPixelColorDepthBits() == PIXEL_COLOR_DEPTH_BITS &&
             PIXEL_COLOR_DEPTH_BITS <= 8;
    for (int v = 0; v < 256; v++) {
        const uint32_t lum = lumConvTab[v];
        uint32_t spread = 0;
        for (int d = 0; d < PIXEL_COLOR_DEPTH_BITS; d++) {
            spread |= ((lum >> d) & 1U) << (3 * d);
        }
        s_spread[v] = spread;
    }
#else
    s_fast = false;
#endif
    ESP_LOGI(TAG, "span writer: %s", s_fast ? "direct" : "per-pixel fallback");
#if !defined(NO_CIE1931) && LUT_NATIVE_BIT_DEPTH && !defined(SPIRAM_DMA_BUFFER)
    {
        /* Where the DMA rows are (internal SRAM banks: 0x3FFAE000.. SRAM2,
         * 0x3FFE0000.. SRAM1): a CPU access to the bank the I2S DMA streams
         * from waits on it, so this is one line of a layout investigation. */
        frameStruct *fb = s_panel->*get(FbTag());
        const uint16_t rows = s_height / MATRIX_ROWS_IN_PARALLEL;
        ESP_LOGI(TAG, "dma rows: %p .. %p", (void *)fb->rowBits[0]->data,
                 (void *)fb->rowBits[rows - 1]->data);
    }
#endif
    s_oe = cfg->oe;
    s_clk = cfg->clk;
    s_driver = cfg->driver;
    s_rtc_oe = cfg->oe >= 0 ? OE_RTC_TAG | static_cast<uint8_t>(cfg->oe) : 0;
    if (esp_register_shutdown_handler(blank_on_shutdown) != ESP_OK) {
        ESP_LOGW(TAG, "no shutdown handler: a restart outside hub75_blank leaves the panel lit");
    }
    s_panel->clearScreen();
    ESP_LOGI(TAG, "panel %ux%u up, refresh %d Hz", s_width, s_height,
             s_panel->calculated_refresh_rate);
    return true;
}

extern "C" void hub75_set_brightness(uint8_t brightness) {
    if (s_panel != nullptr) {
        s_brightness = brightness;
        s_panel->setBrightness8(brightness);
    }
}

extern "C" void hub75_set_clk_phase(bool clkphase) {
    if (s_panel == nullptr || s_stopped || s_clk < 0 ||
        s_driver == HUB75_DRIVER_MBI5124 || s_driver == HUB75_DRIVER_DP3246) {
        return;
    }
    // As Bus_Parallel16::init routes it: the I2S WS output, inverted or not.
#if defined(CONFIG_IDF_TARGET_ESP32S2)
    const uint32_t sig = I2S0O_WS_OUT_IDX;
#elif defined(CONFIG_IDF_TARGET_ESP32S3)
    /* The LCD_CAM's pixel clock (gdma_lcd_parallel16.cpp's pin_wr). */
    const uint32_t sig = LCD_PCLK_IDX;
#else
    const uint32_t sig = I2S1O_WS_OUT_IDX;
#endif
    esp_rom_gpio_connect_out_signal(static_cast<uint32_t>(s_clk), sig, clkphase, false);
    ESP_LOGI(TAG, "clock phase now %s edge", clkphase ? "rising" : "falling");
}

extern "C" void hub75_set_latch_blanking(uint8_t latch_blanking) {
    if (s_panel == nullptr || s_stopped || latch_blanking < 1 || latch_blanking > MAX_LAT_BLANKING) {
        return;
    }
    s_panel->setLatBlanking(latch_blanking);
    s_panel->setBrightness8(s_brightness);
    ESP_LOGI(TAG, "latch blanking now %u", latch_blanking);
}

extern "C" void hub75_clear(void) { HUB75_S3_WRITE_HOOK(0xFFFF) /* (the S3: hub75_shim_s3.inc) */
    if (s_panel != nullptr) {
        s_panel->clearScreen();
    }
}

extern "C" void hub75_draw_rgb888(const uint8_t *rgb, uint16_t w, uint16_t h) { HUB75_S3_WRITE_HOOK(0xFFFF) /* (the S3) */
    if (s_panel == nullptr || rgb == nullptr) {
        return;
    }
    const uint16_t cols = w < s_width ? w : s_width;
    const uint16_t rows = h < s_height ? h : s_height;
    for (uint16_t y = 0; y < rows; y++) {
        const uint8_t *row = rgb + static_cast<size_t>(y) * w * 3;
        for (uint16_t x = 0; x < cols; x++) {
            const uint8_t *px = row + static_cast<size_t>(x) * 3;
            s_panel->drawPixelRGB888(static_cast<int16_t>(x),
                                     static_cast<int16_t>(y), px[0], px[1],
                                     px[2]);
        }
    }
}

extern "C" void hub75_set_pixel(uint16_t x, uint16_t y, uint8_t r, uint8_t g, uint8_t b) { HUB75_S3_WRITE_HOOK(y) /* (the S3) */
    if (s_panel != nullptr) {
        s_panel->drawPixelRGB888(static_cast<int16_t>(x), static_cast<int16_t>(y), r, g, b);
    }
}

/*
 * Exactly what updateMatrixDMABuffer(x, y, r, g, b) does to the frame buffer
 * for every pixel of the span -- same LUT, same bit order, same FIFO-order
 * column swap, same RGB1/RGB2 half -- with everything that does not depend on
 * the pixel hoisted out of the loop: the row's bit-plane base pointer (one
 * shared_ptr and vector dereference per span instead of eight per pixel), the
 * half-panel mask and shift, and the range checks; and the per-plane bit
 * extraction replaced by s_spread. hub75_fb_checksum() is how the two are
 * shown to agree (Panel::bench, probe builds).
 */
/* In IRAM (with hub75_draw_row_pair and hub75_scan_pos): a raced
 * Formula writes a pair between rendering rows, whose flash code evicted
 * these from the flash cache in some builds (psi-perf, 2026-11-02: the
 * same pixels written cost 0.4 ms a frame on main, 1.2 ms on the branch). */
extern "C" IRAM_ATTR void hub75_draw_span(const uint8_t *rgb_row, uint16_t y, uint16_t x0, uint16_t x1) { HUB75_S3_WRITE_HOOK(y) /* (the S3) */
    if (s_panel == nullptr || rgb_row == nullptr || y >= s_height) {
        return;
    }
    if (x1 > s_width) {
        x1 = s_width;
    }
    if (x0 >= x1) {
        return;
    }
    if (!s_fast) {
        for (uint16_t x = x0; x < x1; x++) {
            const uint8_t *px = rgb_row + static_cast<size_t>(x) * 3;
            s_panel->drawPixelRGB888(static_cast<int16_t>(x), static_cast<int16_t>(y), px[0],
                                     px[1], px[2]);
        }
        return;
    }
#if !defined(NO_CIE1931) && LUT_NATIVE_BIT_DEPTH && !defined(SPIRAM_DMA_BUFFER)
    const uint16_t rows_per_frame = s_height / MATRIX_ROWS_IN_PARALLEL;
    uint16_t row = y;
    uint16_t keep = BITMASK_RGB1_CLEAR;
    uint8_t shift = 0;
    if (row >= rows_per_frame) {
        row -= rows_per_frame;
        keep = BITMASK_RGB2_CLEAR;
        shift = BITS_RGB2_OFFSET;
    }
    frameStruct *fb = s_panel->*get(FbTag());
    rowBitStruct *bits = fb->rowBits[row].get();
    const size_t plane = bits->width; /* words per colour-depth plane */
    ESP32_I2S_DMA_STORAGE_TYPE *const base = bits->data;
    const uint8_t depth = PIXEL_COLOR_DEPTH_BITS;

    const uint8_t *px = rgb_row + static_cast<size_t>(x0) * 3;
    for (uint16_t x = x0; x < x1; x++, px += 3) {
        uint32_t bits = s_spread[px[0]] | (s_spread[px[1]] << 1) | (s_spread[px[2]] << 2);
#if defined(ESP32_THE_ORIG)
        const uint16_t col = x ^ 1U; /* ESP32_TX_FIFO_POSITION_ADJUST */
#else
        const uint16_t col = x;
#endif
        ESP32_I2S_DMA_STORAGE_TYPE *w = base + col;
        for (uint8_t d = 0; d < depth; d++, bits >>= 3, w += plane) {
            *w = static_cast<ESP32_I2S_DMA_STORAGE_TYPE>((*w & keep) | ((bits & 7U) << shift));
        }
    }
#endif
}

/*
 * Rows y and y + height/2 in full, which share every DMA word (RGB1 and RGB2
 * are two halves of the same uint16_t): one read-modify-write per word
 * instead of two. For frames that repaint whole rows -- the first of every
 * file, scrollers. Falls back to two spans when the fast path is off.
 */
extern "C" IRAM_ATTR void hub75_draw_row_pair(const uint8_t *top_row, const uint8_t *bottom_row, uint16_t y) { HUB75_S3_WRITE_HOOK(y) /* (the S3) */
    if (s_panel == nullptr || top_row == nullptr || bottom_row == nullptr) {
        return;
    }
    const uint16_t rows_per_frame = s_height / MATRIX_ROWS_IN_PARALLEL;
    if (y >= rows_per_frame) {
        return;
    }
    if (!s_fast) {
        hub75_draw_span(top_row, y, 0, s_width);
        hub75_draw_span(bottom_row, static_cast<uint16_t>(y + rows_per_frame), 0, s_width);
        return;
    }
#if !defined(NO_CIE1931) && LUT_NATIVE_BIT_DEPTH && !defined(SPIRAM_DMA_BUFFER)
    frameStruct *fb = s_panel->*get(FbTag());
    rowBitStruct *bits = fb->rowBits[y].get();
    const size_t plane = bits->width;
    ESP32_I2S_DMA_STORAGE_TYPE *const base = bits->data;
    const uint8_t depth = PIXEL_COLOR_DEPTH_BITS;
    const uint8_t *t = top_row;
    const uint8_t *b = bottom_row;
    for (uint16_t x = 0; x < s_width; x++, t += 3, b += 3) {
        uint32_t hi = s_spread[t[0]] | (s_spread[t[1]] << 1) | (s_spread[t[2]] << 2);
        uint32_t lo = s_spread[b[0]] | (s_spread[b[1]] << 1) | (s_spread[b[2]] << 2);
#if defined(ESP32_THE_ORIG)
        const uint16_t col = x ^ 1U;
#else
        const uint16_t col = x;
#endif
        ESP32_I2S_DMA_STORAGE_TYPE *w = base + col;
        for (uint8_t d = 0; d < depth; d++, hi >>= 3, lo >>= 3, w += plane) {
            *w = static_cast<ESP32_I2S_DMA_STORAGE_TYPE>(
                (*w & BITMASK_RGB12_CLEAR) | (hi & 7U) | ((lo & 7U) << BITS_RGB2_OFFSET));
        }
    }
#endif
}

/*
 * FNV-1a over every word of the frame buffer being written, all rows and all
 * colour-depth planes. A self-test: draw the same image through
 * drawPixelRGB888 and through hub75_draw_span, and the two must agree.
 */
extern "C" uint32_t hub75_fb_checksum(void) {
    if (s_panel == nullptr) {
        return 0;
    }
    frameStruct *fb = s_panel->*get(FbTag());
    uint32_t h = 2166136261u;
    for (auto &rb : fb->rowBits) {
        const size_t n = rb->width * rb->colour_depth;
        for (size_t i = 0; i < n; i++) {
            h = (h ^ rb->data[i]) * 16777619u;
        }
    }
    return h;
}

extern "C" void hub75_flip(void) { HUB75_S3_FLIP_HOOK /* (the S3: hub75_shim_s3.inc) */
    if (s_panel != nullptr) {
        s_panel->flipDMABuffer();
    }
}

extern "C" uint8_t hub75_refresh_rate(void) {
    if (s_panel == nullptr) {
        return 0;
    }
    int rate = s_panel->calculated_refresh_rate;
    if (rate < 0) {
        return 0;
    }
    if (rate > 255) {
        return 255;
    }
    return static_cast<uint8_t>(rate);
}

void hub75_dma_span(uint32_t *lo, uint32_t *hi, uint32_t *bytes, uint32_t *n, uint32_t *dlo, uint32_t *dhi)
{
    *lo = *hi = *bytes = *n = *dlo = *dhi = 0;
    if (s_panel == nullptr) {
        return;
    }
    frameStruct *fb = s_panel->*get(FbTag());
    if (fb != nullptr) {
        uint32_t l = UINT32_MAX, h = 0;
        for (auto &r : fb->rowBits) {
            if (!r || r->data == nullptr) {
                continue;
            }
            const uint32_t a = (uint32_t)(uintptr_t)r->data;
            const uint32_t sz = (uint32_t)r->getColorDepthSize(false);
            if (a < l) l = a;
            if (a + sz > h) h = a + sz;
            *bytes += sz;
            *n += 1;
        }
        if (*n) {
            *lo = l;
            *hi = h;
        }
    }
    Bus_Parallel16 &bus = s_panel->*get(BusTag());
    HUB75_DMA_DESCRIPTOR_T *desc = bus.*get(DescTag());
    const uint32_t count = bus.*get(DescCountTag());
    if (desc != nullptr && count) {
        *dlo = (uint32_t)(uintptr_t)desc;
        *dhi = *dlo + count * sizeof(HUB75_DMA_DESCRIPTOR_T);
    }
}

#if defined(CONFIG_IDF_TARGET_ESP32S3)
/*
 * --- The ESP32-S3 (hub75_shim_s3.inc declares these for the code above) ---
 *
 * The scan model on the S3. The library builds the same descriptor ring as
 * on the classic (configureDMA: one descriptor of every colour plane per row
 * pair, then one per extra pass of the higher planes; dma_descriptor_t, at
 * most 4092 B each), fed to the LCD_CAM by a GDMA out-channel. The channel's
 * registers name descriptors in the ring: OUT_DSCR ("the current outlink
 * descriptor"), OUT_DSCR_BF0 and _BF1 (the last finished and the one before,
 * by the TRM) and OUT_STATE's 18 low address bits. Which of them is the one
 * being sent on silicon is the bench's to say (s3_pins_check); the racer
 * reads one (hub75_scan_set_source, OUT_DSCR by default).
 */

/* Bus_Parallel16::dma_chan (private), by the idiom above. */
struct DmaChanTag {
    typedef gdma_channel_handle_t Bus_Parallel16::*type;
    friend type get(DmaChanTag);
};
template struct Rob<DmaChanTag, &Bus_Parallel16::dma_chan>;

/* The LCD's GDMA channel (gdma_get_channel_id), -1 until s3_scan_init. */
static int s_gdma_ch = -1;
static uint8_t s_scan_src = HUB75_S3_SRC_DSCR;
static bool s_pins_check = false;
static bool s_power_safe = false;
/* A, B, C, D's GPIOs (hub75_init's config), for s3_pins_check. */
static int8_t s_addr_pins[4] = {-1, -1, -1, -1};

static const char *const S3_SRC_NAME[4] = {"OUT_DSCR", "OUT_DSCR_BF0", "OUT_DSCR_BF1", "OUT_STATE"};

extern "C" void hub75_scan_set_source(uint8_t source) {
    s_scan_src = source <= HUB75_S3_SRC_STATE ? source : HUB75_S3_SRC_DSCR;
}

extern "C" void hub75_scan_set_pins_check(bool on) {
    s_pins_check = on;
}

extern "C" void hub75_set_power_safe(bool on) {
    s_power_safe = on;
}

/* --- Double buffering (hub75_set_buffers; hub75_shim.h) --- */

/* MatrixPanel_I2S_DMA::frame_buffer and ::back_buffer_id, and
 * Bus_Parallel16::_dmadesc_b (all private), by the idiom above. */
struct FrameBufTag {
    typedef frameStruct (MatrixPanel_I2S_DMA::*type)[2];
    friend type get(FrameBufTag);
};
template struct Rob<FrameBufTag, &MatrixPanel_I2S_DMA::frame_buffer>;
struct BackIdTag {
    typedef volatile int MatrixPanel_I2S_DMA::*type;
    friend type get(BackIdTag);
};
template struct Rob<BackIdTag, &MatrixPanel_I2S_DMA::back_buffer_id>;
struct DescBTag {
    typedef HUB75_DMA_DESCRIPTOR_T *Bus_Parallel16::*type;
    friend type get(DescBTag);
};
template struct Rob<DescBTag, &Bus_Parallel16::_dmadesc_b>;

/* Internal heap the second frame buffer and ring must leave free at
 * hub75_init (WiFi, lwIP, httpd, the decoders and the Formula compiler come
 * after it): below this the panel runs single-buffered and the beam is
 * raced. */
#define S3_DBUF_KEEP (96 * 1024)

/* What hub75_set_buffers asked (0: the build's default). */
static uint8_t s_buffers_asked = 0;
/* The build's default: double-buffered where WiFi's and lwIP's buffers go to
 * PSRAM first (the carrier image's all-psram switches: ~110-150 KB of the
 * internal heap free while playing, single-buffered, on the bench), single
 * where they stay in internal RAM (the plain S3 build: ~38 KB free while
 * playing single-buffered with no WiFi, in the emulator; the second buffer
 * and ring take ~38 KB of it). */
#if CONFIG_SPIRAM_TRY_ALLOCATE_WIFI_LWIP
#define S3_BUILD_BUFFERS 2
#else
#define S3_BUILD_BUFFERS 1
#endif
/* A nested hub75_init (the hook's double-buffered try) is running. */
static bool s_in_init = false;
/* Running double-buffered: set once at init, never cleared. */
static bool s_dbuf = false;
/* The two descriptor rings' first descriptor (0: frame_buffer[0]'s, the
 * library's _dmadesc_a; 1: _dmadesc_b's), their length. */
static uint32_t s_ring[2] = {0, 0};
static uint32_t s_ring_count = 0;
/* The ring the DMA shows, or was last asked to (the last flip's). */
static uint8_t s_front = 0;
/* A flip asked, its switch not seen yet. */
static bool s_flip_pending = false;
/* When the last flip was asked, and the us then left of the ring the DMA
 * was in (its switch is due then). */
static int64_t s_flip_at = 0;
static uint32_t s_flip_left_us = 0;
/* The descriptor the DMA was on at the flip (for the late line). */
static uint32_t s_flip_idx = 0;
/* When the last switch was (est.), for the shown_* histogram; 0 none. */
static int64_t s_last_switch = 0;
/* The buffer check's red variant: the next resolve skips this pair's copy
 * (UINT8_MAX: none). */
static uint8_t s_skip_copy = UINT8_MAX;
/* Test builds' forced fallbacks (hub75_s3_set_test). */
static uint8_t s_test = HUB75_S3_TEST_NONE;
/* Row pairs written since the last flip, and those to copy across once its
 * switch is seen (bit p: pair p). */
static uint32_t s_written = 0;
static uint32_t s_copy = 0;
static hub75_s3_dbuf_t s_db = {};

extern "C" void hub75_set_buffers(uint8_t buffers) {
    s_buffers_asked = buffers == 1 || buffers == 2 ? buffers : 0;
}

extern "C" void hub75_s3_dbuf(hub75_s3_dbuf_t *out) {
    if (out != nullptr) {
        *out = s_db;
        out->build_buffers = S3_BUILD_BUFFERS;
    }
}

extern "C" void hub75_s3_set_test(uint8_t test) {
    s_test = test;
}

extern "C" void hub75_s3_skip_copy(uint8_t pair) {
    s_skip_copy = pair;
}

static uint32_t s3_resolve(void);

/* FNV-1a over one frame buffer (every row pair, every plane). */
static uint32_t s3_fb_sum(const frameStruct &fb) {
    uint32_t h = 2166136261u;
    for (auto &rb : fb.rowBits) {
        const size_t n = rb->width * rb->colour_depth;
        for (size_t i = 0; i < n; i++) {
            h = (h ^ rb->data[i]) * 16777619u;
        }
    }
    return h;
}

extern "C" bool hub75_s3_fb_sums(uint32_t *front, uint32_t *back) {
    if (!s_dbuf || s_panel == nullptr || front == nullptr || back == nullptr) {
        return false;
    }
    if (s_flip_pending) {
        s3_resolve();
    }
    frameStruct *fbs = s_panel->*get(FrameBufTag());
    *front = s3_fb_sum(fbs[s_front]);
    *back = s3_fb_sum(fbs[s_front ^ 1]);
    return true;
}

static uint32_t s3_internal_free(void) {
    return static_cast<uint32_t>(heap_caps_get_free_size(MALLOC_CAP_INTERNAL));
}
static uint32_t s3_internal_largest(void) {
    return static_cast<uint32_t>(heap_caps_get_largest_free_block(MALLOC_CAP_INTERNAL));
}

/* The LCD's GDMA channel (s_gdma_ch), from the panel's bus; -1 none. */
static int s3_find_gdma(void) {
    if (s_panel == nullptr) {
        return -1;
    }
    Bus_Parallel16 &bus = s_panel->*get(BusTag());
    const gdma_channel_handle_t chan = bus.*get(DmaChanTag());
    int ch = -1;
    if (chan == nullptr || gdma_get_channel_id(chan, &ch) != ESP_OK || ch < 0 ||
        ch >= SOC_GDMA_PAIRS_PER_GROUP) {
        return -1;
    }
    s_gdma_ch = ch;
    return ch;
}

/* Which ring GDMA's OUT_DSCR is in (0, 1; -1 neither, or no channel), and
 * the descriptor's index in it. In IRAM: every write calls it. */
static IRAM_ATTR int s3_ring_now(uint32_t *idx) {
    if (s_gdma_ch < 0 || s_ring_count == 0) {
        return -1;
    }
    const uint32_t reg = GDMA.channel[s_gdma_ch].out.dscr;
    for (int r = 0; r < 2; r++) {
        const uint32_t i = (reg - s_ring[r]) / sizeof(HUB75_DMA_DESCRIPTOR_T);
        if (i < s_ring_count) {
            *idx = i;
            return r;
        }
    }
    return -1;
}

/*
 * The last flip's switch: wait until OUT_DSCR is in the ring it asked for
 * (a tick asleep at a time while the old ring has more than 1.5 ms to go,
 * else spinning), and past that ring's first two descriptors (OUT_DSCR
 * leads the pins: the old ring's last bytes may still be on their way out
 * when it already names the new one: at a later lap's wrap this costs a
 * spin of two descriptors too). Late: the DMA was seen still in the old ring
 * more than 1 ms past the end of the refresh the flip was asked in. Then the pairs the flip's frame wrote
 * are copied from the new front buffer into the new back one, which held the
 * frame before. Given up (counted) 3 refreshes after the flip; at once if
 * the DMA was stopped (hub75_blank). Returns the us waited (0: no wait).
 */
static uint32_t s3_resolve(void) {
    const int64_t t0 = esp_timer_get_time();
    const uint32_t per = s_db.refresh_us != 0 ? s_db.refresh_us : 20000;
    int64_t give_up = s_flip_at + 3 * static_cast<int64_t>(per) + 2000;
    if (give_up < t0 + 1000) {
        give_up = t0 + 1000;
    }
    bool waited = false;
    bool old_seen = false; /* the DMA seen still in the ring it was leaving */
    int64_t old_at = 0;    /* and when, last */
    bool seen = false;
    int64_t now = t0;
    while (!s_stopped) {
        uint32_t idx = 0;
        const int ring = s3_ring_now(&idx);
        now = esp_timer_get_time();
        if (ring == s_front && idx >= 2) {
            seen = true;
            break;
        }
        if (now > give_up) {
            s_db.timeouts++;
            break;
        }
        waited = true;
        if (ring >= 0 && ring != s_front) {
            old_seen = true;
            old_at = now;
        }
        const uint32_t left = (ring >= 0 && ring != s_front) ? (s_ring_count - idx) * per / s_ring_count : 0;
        if (left > 1500) {
            vTaskDelay(1);
        }
    }
    if (seen) {
        s_db.switched++;
        // Late: the DMA still seen in the old ring more than 1 ms past the
        // end of the refresh the flip was asked in (the last sighting, not
        // the first look at the new ring: a sleep between them is not it).
        if (old_seen && old_at > s_flip_at + static_cast<int64_t>(s_flip_left_us) + 1000) {
            // Asked in the ring's last two descriptors: the DMA may have
            // read the last one's link already (a refresh more of the old
            // frame, expected). Anywhere else it should not happen.
            const bool wrap = s_flip_idx != UINT32_MAX && s_flip_idx + 2 >= s_ring_count;
            if (wrap) {
                s_db.late_wrap++;
            } else {
                s_db.late++;
            }
            if (s_db.late + s_db.late_wrap <= 8) {
                ESP_LOGW(TAG, "panel: a flip's switch late: asked at descriptor %u of %u, due in %u us, the old ring still seen after %u us",
                         (unsigned)s_flip_idx, (unsigned)s_ring_count, (unsigned)s_flip_left_us,
                         (unsigned)(old_at - s_flip_at));
            }
        }
        // The refreshes the frame before was shown: from its switch to this
        // one. This one's time: between the last sighting of the old ring
        // and now when it was watched, else when it was due (the flip's
        // descriptor's time to the ring's end).
        const int64_t sw = old_seen ? (old_at + now) / 2 : s_flip_at + s_flip_left_us;
        if (s_last_switch != 0 && sw > s_last_switch) {
            const uint32_t n = static_cast<uint32_t>((sw - s_last_switch + per / 2) / per);
            if (n <= 1) {
                s_db.shown_1++;
            } else if (n == 2) {
                s_db.shown_2++;
            } else {
                s_db.shown_3plus++;
            }
        }
        s_last_switch = sw;
    }
    const uint32_t copy = s_copy;
    if (copy != 0 && !s_stopped) {
        frameStruct *fbs = s_panel->*get(FrameBufTag());
        const frameStruct &from = fbs[s_front];
        frameStruct &to = fbs[s_front ^ 1];
        const size_t rows = from.rowBits.size();
        for (size_t p = 0; p < rows && p < 32; p++) {
            if ((copy >> p) & 1U) {
                if (p == s_skip_copy) { /* (the buffer check's red variant: hub75_s3_skip_copy) */
                    s_skip_copy = UINT8_MAX;
                    continue;
                }
                memcpy(to.rowBits[p]->data, from.rowBits[p]->data, from.rowBits[p]->getColorDepthSize(false));
                s_db.copied_pairs++;
            }
        }
    }
    s_copy = 0;
    s_flip_pending = false;
    if (!waited) {
        return 0;
    }
    const uint32_t us = static_cast<uint32_t>(esp_timer_get_time() - t0);
    s_db.waits++;
    s_db.wait_us += us;
    if (us > s_db.wait_max_us) {
        s_db.wait_max_us = us;
    }
    return us;
}

extern "C" uint32_t hub75_s3_back_ready(void) {
    if (!s_dbuf || !s_flip_pending) {
        return 0;
    }
    return s3_resolve();
}

static IRAM_ATTR void s3_before_write(uint32_t row) {
    if (!s_dbuf) {
        return;
    }
    if (s_flip_pending) {
        s3_resolve();
    }
    const uint32_t pairs = s_height / MATRIX_ROWS_IN_PARALLEL;
    s_written |= row < s_height ? (1U << (row % pairs)) : (pairs >= 32 ? UINT32_MAX : (1U << pairs) - 1);
    // The check the double buffering is for (the emulator lane's
    // `back_hits`): the DMA must not be in the ring about to be written.
    uint32_t idx = 0;
    if (!s_stopped && s3_ring_now(&idx) == (s_front ^ 1)) {
        s_db.back_hits++;
    }
}

static bool s3_flip(void) {
    if (!s_dbuf) {
        return false; /* (single-buffered: the library's flip is a no-op) */
    }
    if (s_stopped) {
        return true;
    }
    // Nothing written since the last flip (whose switch then is still due):
    // nothing new to show, and flipping back would show the older frame.
    if (s_flip_pending || s_written == 0) {
        s_db.skipped++;
        return true;
    }
    const uint8_t back = static_cast<uint8_t>(s_panel->*get(BackIdTag()) & 1);
    uint32_t idx = 0;
    const int ring = s3_ring_now(&idx);
    s_flip_idx = ring >= 0 ? idx : UINT32_MAX;
    s_flip_left_us = (ring >= 0 && s_db.refresh_us != 0)
                         ? (s_ring_count - idx) * s_db.refresh_us / s_ring_count
                         : s_db.refresh_us;
    // The library's flip: the ring now shown ends into the back buffer's,
    // which loops on itself; the library writes the other buffer from here.
    s_panel->flipDMABuffer();
    s_flip_at = esp_timer_get_time();
    s_front = back;
    s_flip_pending = true;
    s_copy = s_written;
    s_written = 0;
    s_db.flips++;
    return true;
}

/* The heap hub75_init's frame buffers and descriptors take, at most, for one
 * buffer (before begin(), which says exactly: logged after). */
static uint32_t s3_one_buffer_bytes(const hub75_cfg_t *cfg) {
    const uint32_t width = static_cast<uint32_t>(cfg->mx_width) * cfg->chain_length;
    const uint32_t rows = cfg->mx_height / MATRIX_ROWS_IN_PARALLEL;
    const uint32_t fb = rows * width * PIXEL_COLOR_DEPTH_BITS * sizeof(ESP32_I2S_DMA_STORAGE_TYPE);
    /* One descriptor a colour plane's pass, at most 2^(depth-1) passes a
     * pair: 64 a pair is the default panel's 32 with room. */
    const uint32_t desc = rows * 64 * sizeof(HUB75_DMA_DESCRIPTOR_T);
    return fb + desc;
}

/* The double-buffered init: a nested hub75_init with double_buff, checked. */
static bool s3_init_double(hub75_cfg_t *out) {
    const uint32_t one = s3_one_buffer_bytes(out);
    const uint32_t free_dma = static_cast<uint32_t>(heap_caps_get_free_size(MALLOC_CAP_INTERNAL | MALLOC_CAP_DMA));
    const uint32_t largest = static_cast<uint32_t>(heap_caps_get_largest_free_block(MALLOC_CAP_INTERNAL | MALLOC_CAP_DMA));
    const bool forced = s_test == HUB75_S3_TEST_HEAP;
    if (forced || free_dma < 2 * one + S3_DBUF_KEEP || largest < one / 4) {
        s_db.why = HUB75_S3_WHY_HEAP;
        ESP_LOGW(TAG,
                 "panel: single-buffered, the beam raced: double buffering needs up to %u B of internal DMA heap "
                 "and %u B left after it, %u B free (largest block %u)%s",
                 (unsigned)(2 * one), (unsigned)S3_DBUF_KEEP, (unsigned)free_dma, (unsigned)largest,
                 forced ? " (forced: dbuf_test heap)" : "");
        return false;
    }
    out->double_buff = true;
    s_in_init = true;
    const bool ok = hub75_init(out);
    s_in_init = false;
    out->double_buff = false;
    if (!ok) {
        s_db.why = HUB75_S3_WHY_BEGIN;
        ESP_LOGE(TAG, "panel: the double-buffered start failed (internal heap %u B free): single-buffered, the beam raced",
                 (unsigned)s3_internal_free());
        return false;
    }
    Bus_Parallel16 &bus = s_panel->*get(BusTag());
    HUB75_DMA_DESCRIPTOR_T *a = bus.*get(DescTag());
    HUB75_DMA_DESCRIPTOR_T *b = bus.*get(DescBTag());
    const uint32_t count = bus.*get(DescCountTag());
    const bool forced_ring = s_test == HUB75_S3_TEST_RING;
    if (forced_ring || a == nullptr || b == nullptr || count == 0 || !s_panel->getCfg().double_buff || s3_find_gdma() < 0) {
        // Not reached in practice for a missing second ring: when its
        // allocation fails, the library clears _double_dma_buffer and then
        // writes buffer 1's descriptors through the null _dmadesc_b in
        // setupDMA (create_dma_desc_link), before begin() returns. What
        // keeps that from happening is the heap check above. This catches
        // the rest (no GDMA channel id), and is the path test builds force
        // (dbuf_test ring) so the start-again-single-buffered runs: the
        // driver stopped and taken apart, then hub75_init's own body. The
        // library's release() frees only the first ring and the i80 bus:
        // the second ring and the GDMA channel go here, and the LCD's
        // lcd_start is cleared (the next init spins on it: the emulator's
        // lane hung there before this).
        s_db.why = HUB75_S3_WHY_RING;
        ESP_LOGE(TAG, "panel: double-buffered start without a second descriptor ring or GDMA channel%s: "
                      "single-buffered, the beam raced",
                 forced_ring ? " (forced: dbuf_test ring)" : "");
        esp_unregister_shutdown_handler(blank_on_shutdown);
        const gdma_channel_handle_t chan = bus.*get(DmaChanTag());
        s_panel->stopDMAoutput();
        LCD_CAM.lcd_user.lcd_start = 0;
        LCD_CAM.lcd_user.lcd_update = 1;
        if (chan != nullptr) {
            gdma_disconnect(chan);
            gdma_del_channel(chan);
        }
        delete s_panel; /* (release(): the first ring, the i80 bus; the frame buffers) */
        s_panel = nullptr;
        if (b != nullptr) {
            heap_caps_free(b);
        }
        s_gdma_ch = -1;
        return false;
    }
    s_ring[0] = reinterpret_cast<uint32_t>(a);
    s_ring[1] = reinterpret_cast<uint32_t>(b);
    s_ring_count = count;
    frameStruct *fbs = s_panel->*get(FrameBufTag());
    uint32_t fb_bytes = 0;
    for (auto &r : fbs[0].rowBits) {
        fb_bytes += static_cast<uint32_t>(r->getColorDepthSize(false));
    }
    s_db.fb_bytes = fb_bytes;
    s_db.desc_bytes = count * sizeof(HUB75_DMA_DESCRIPTOR_T);
    // begin() flipped once: the DMA loops on ring 0 and the library writes
    // buffer 1.
    s_front = 0;
    s_written = 0;
    s_dbuf = true;
    s_db.buffers = 2;
    s_db.why = HUB75_S3_WHY_OK;
    return true;
}

static bool s3_init_hook(const hub75_cfg_t *cfg, hub75_cfg_t *out, bool *result) {
    if (cfg == nullptr) {
        *result = false;
        return true;
    }
    *out = *cfg;
    if (s_in_init || s_panel != nullptr) {
        return false; /* (the nested try; or a second call: a no-op as ever) */
    }
    s_db.buffers = 1;
    s_db.heap_before = s3_internal_free();
    s_db.largest_before = s3_internal_largest();
    s_addr_pins[0] = cfg->a;
    s_addr_pins[1] = cfg->b;
    s_addr_pins[2] = cfg->c;
    s_addr_pins[3] = cfg->d;
    if (!s_power_safe) {
        if ((s_buffers_asked != 0 ? s_buffers_asked : S3_BUILD_BUFFERS) < 2) {
            s_db.why = s_buffers_asked != 0 ? HUB75_S3_WHY_ASKED : HUB75_S3_WHY_BUILD;
            return false;
        }
        if (s3_init_double(out)) {
            *result = true;
            return true;
        }
        return false; /* (single-buffered: hub75_init's own body, with *out) */
    }
    // No driver, no DMA, and nothing driven into a panel that may have no
    // supply (the bench's brownout loops, 2026-11-13: a driven line into an
    // unpowered panel feeds it through its input clamps, ~20-40 mA a line):
    // every HUB75 pin an input with no output path, OE pulled up (the
    // panel's off level) by the pad's own pull-up, not driven high. Every
    // other hub75_* call is a no-op with no panel. (hub75_early_blank has
    // driven OE high from start-up to here, ~1 s, as on every boot; and
    // hub75_blank, at a restart, does again until that restart.)
    const int8_t pins[] = {cfg->r1, cfg->g1, cfg->b1, cfg->r2, cfg->g2, cfg->b2, cfg->a,
                           cfg->b,  cfg->c,  cfg->d,  cfg->e,  cfg->lat, cfg->clk, cfg->oe};
    for (const int8_t pin : pins) {
        if (pin < 0 || !oe_pin_ok(static_cast<uint32_t>(pin))) {
            continue;
        }
        const uint32_t p = static_cast<uint32_t>(pin);
        gpio_ll_output_disable(&GPIO, p);
        esp_rom_gpio_pad_select_gpio(p);
        gpio_ll_input_enable(&GPIO, p);
        gpio_ll_pulldown_dis(&GPIO, p);
        if (pin == cfg->oe) {
            gpio_ll_pullup_en(&GPIO, p);
        } else {
            gpio_ll_pullup_dis(&GPIO, p);
        }
    }
    s_oe = cfg->oe;
    s_db.why = HUB75_S3_WHY_NO_PANEL;
    ESP_LOGW(TAG, "power-safe: the panel driver is not started (every HUB75 pin an input, OE pulled up, nothing driven)");
    *result = true;
    return true;
}

/* A register's value as a descriptor index (UINT32_MAX outside the ring).
 * No switch: a jump table would be flash rodata, and this runs from IRAM. */
static IRAM_ATTR uint32_t s3_index_of(uint8_t src, uint32_t reg) {
    uint32_t off = reg - s_scan_desc;
    if (src == HUB75_S3_SRC_STATE) {
        off &= 0x3FFFFu; /* 18 address bits: rebased onto the ring's */
    }
    const uint32_t idx = off / sizeof(HUB75_DMA_DESCRIPTOR_T);
    return idx < s_scan_count ? idx : UINT32_MAX;
}

static IRAM_ATTR uint32_t s3_reg(uint8_t src) {
    if (src == HUB75_S3_SRC_BF0) {
        return GDMA.channel[s_gdma_ch].out.dscr_bf0;
    }
    if (src == HUB75_S3_SRC_BF1) {
        return GDMA.channel[s_gdma_ch].out.dscr_bf1;
    }
    if (src == HUB75_S3_SRC_STATE) {
        return GDMA.channel[s_gdma_ch].out.state.dscr_addr;
    }
    return GDMA.channel[s_gdma_ch].out.dscr;
}

/* In IRAM: hub75_scan_pos (IRAM) calls it while a Formula races the beam. */
static IRAM_ATTR uint32_t s3_scan_desc_index(void) {
    if (s_gdma_ch < 0) {
        return UINT32_MAX;
    }
    return s3_index_of(s_scan_src, s3_reg(s_scan_src));
}

/*
 * Which register is the descriptor being sent (S3 port phase 3, A2; bench
 * and test builds only, hub75_scan_set_pins_check): the address lines A-D
 * carry the row pair being shown (the first colour plane of a pair's first
 * descriptor the pair before: the library's latch overlap), so each
 * register's descriptor, as a row pair, is set beside the pins read back
 * through their input path (enabled here; the LCD_CAM's output is
 * untouched), a few hundred times over two refreshes or so. "pins a row
 * ahead": the register lags the scan; "a row behind": it leads.
 *
 * What the bench should see (the default panel: 32 descriptors a pair, the
 * first one every colour plane, the pins carrying the pair before for its
 * first plane): the register that names the descriptor being sent agrees
 * ~97% of the time, its misses counted as "ahead" (that first plane); one a
 * descriptor early ~95%, with ~13 of 500 "behind"; one a descriptor late
 * ~80%. The racer's register should be the ~97% one (NVS scan_reg).
 *
 * In the emulator the number means nothing: esp32sim drains the descriptors
 * but does not loop the LCD_CAM's output back to GPIO_IN, so the pins read
 * a constant and every register "agrees" ~1/16 of the time (~18-22/500).
 * The emulator lanes check only that the lines are logged.
 */
static void s3_pins_check(uint32_t per_row) {
    for (int b = 0; b < 4; b++) {
        if (s_addr_pins[b] >= 0 && s_addr_pins[b] < 32) {
            gpio_ll_input_enable(&GPIO, static_cast<uint32_t>(s_addr_pins[b]));
        }
    }
    const uint32_t n = 500;
    uint32_t agree[4] = {0, 0, 0, 0}, ahead[4] = {0, 0, 0, 0}, behind[4] = {0, 0, 0, 0},
             unread[4] = {0, 0, 0, 0};
    for (uint32_t i = 0; i < n; i++) {
        uint32_t r[4];
        r[0] = s3_reg(HUB75_S3_SRC_DSCR);
        r[1] = s3_reg(HUB75_S3_SRC_BF0);
        const uint32_t in = REG_READ(GPIO_IN_REG);
        r[2] = s3_reg(HUB75_S3_SRC_BF1);
        r[3] = s3_reg(HUB75_S3_SRC_STATE);
        uint32_t addr = 0;
        for (int b = 0; b < 4; b++) {
            if (s_addr_pins[b] >= 0 && s_addr_pins[b] < 32) {
                addr |= ((in >> s_addr_pins[b]) & 1U) << b;
            }
        }
        for (uint8_t s = 0; s < 4; s++) {
            const uint32_t idx = s3_index_of(s, r[s]);
            if (idx == UINT32_MAX) {
                unread[s]++;
                continue;
            }
            const uint32_t row = (idx / per_row) & 15U;
            if (row == addr) {
                agree[s]++;
            } else if (((row + 1) & 15U) == addr) {
                ahead[s]++;
            } else if (((addr + 1) & 15U) == row) {
                behind[s]++;
            }
        }
        esp_rom_delay_us(37); /* (prime to the ~625 us row pair) */
    }
    // One line a register. Bench: the "current" register ~97% (misses as
    // "ahead"), one a descriptor early ~95% (~13 "behind"), one late ~80%;
    // the emulator: ~1/16 for all four, meaningless (no pin loopback).
    for (uint8_t s = 0; s < 4; s++) {
        ESP_LOGI(TAG,
                 "scan: register %s agrees with the address pins %u/%u (pins a row ahead %u, a row "
                 "behind %u, outside the ring %u)%s",
                 S3_SRC_NAME[s], (unsigned)agree[s], (unsigned)n, (unsigned)ahead[s],
                 (unsigned)behind[s], (unsigned)unread[s], s == s_scan_src ? ": the racer's" : "");
    }
}

/* hub75_scan_init when double-buffered: no beam to race (hub75_scan_pos
 * stays unknown), but the refresh is timed off ring 0, which the DMA loops
 * until the first flip (four wraps, the shortest of three intervals), for
 * the flips' timing; then the line the lanes and the bench read. */
static void s3_dbuf_scan_init(void) {
    int64_t wraps[4] = {0, 0, 0, 0};
    int seen = 0;
    uint32_t last = UINT32_MAX;
    const int64_t give_up = esp_timer_get_time() + 150000;
    while (seen < 4 && esp_timer_get_time() < give_up) {
        uint32_t idx = 0;
        if (s3_ring_now(&idx) != 0) {
            continue;
        }
        if (last != UINT32_MAX && idx + s_ring_count / 2 < last) {
            wraps[seen++] = esp_timer_get_time();
        }
        last = idx;
    }
    uint32_t period = 0;
    if (seen == 4) {
        period = UINT32_MAX;
        for (int i = 1; i < 4; i++) {
            const uint32_t d = static_cast<uint32_t>(wraps[i] - wraps[i - 1]);
            period = d < period ? d : period;
        }
        const int calc = s_panel->calculated_refresh_rate;
        const uint32_t expect = calc > 0 ? 1000000u / static_cast<uint32_t>(calc) : 0;
        if (expect == 0 || period * 3 < expect * 2 || period > expect * 2) {
            ESP_LOGW(TAG, "panel: refresh %u us measured, %u us expected: implausible, not used",
                     (unsigned)period, (unsigned)expect);
            period = 0;
        }
    } else {
        ESP_LOGW(TAG, "panel: no refresh of ring 0 in 150 ms: flips timed out at 60 ms");
    }
    s_db.refresh_us = period;
    ESP_LOGI(TAG,
             "panel: double-buffered: 2 frame buffers of %u B and 2 descriptor rings of %u B (%u descriptors), "
             "refresh %u us (GDMA channel %d; a flip's switch seen on OUT_DSCR): the beam is not raced; "
             "internal heap %u B free (largest %u) before the driver, %u (%u) after",
             (unsigned)s_db.fb_bytes, (unsigned)s_db.desc_bytes, (unsigned)s_ring_count, (unsigned)period,
             s_gdma_ch, (unsigned)s_db.heap_before, (unsigned)s_db.largest_before, (unsigned)s_db.heap_after,
             (unsigned)s_db.largest_after);
}

/* hub75_scan_init on the S3: the classic's layout check and wrap timing,
 * through the GDMA channel's register. */
static void s3_scan_init(void) {
    s_db.heap_after = s3_internal_free();
    s_db.largest_after = s3_internal_largest();
    if (s_dbuf) {
        s3_dbuf_scan_init();
        return;
    }
    ESP_LOGI(TAG, "panel: single-buffered (%s): internal heap %u B free (largest %u) before the driver, %u (%u) after",
             s_db.why == HUB75_S3_WHY_ASKED  ? "asked"
             : s_db.why == HUB75_S3_WHY_BUILD ? "the build's default: WiFi's buffers in internal RAM"
                                              : "double buffering did not start",
             (unsigned)s_db.heap_before, (unsigned)s_db.largest_before, (unsigned)s_db.heap_after,
             (unsigned)s_db.largest_after);
    const uint16_t rows = static_cast<uint16_t>(s_height / MATRIX_ROWS_IN_PARALLEL);
    if (s_panel->getCfg().double_buff || rows == 0) {
        return;
    }
    Bus_Parallel16 &bus = s_panel->*get(BusTag());
    HUB75_DMA_DESCRIPTOR_T *desc = bus.*get(DescTag());
    const uint32_t count = bus.*get(DescCountTag());
    const gdma_channel_handle_t chan = bus.*get(DmaChanTag());
    int ch = -1;
    if (chan == nullptr || gdma_get_channel_id(chan, &ch) != ESP_OK || ch < 0 ||
        ch >= SOC_GDMA_PAIRS_PER_GROUP) {
        ESP_LOGW(TAG, "scan: no GDMA channel for the LCD: the beam is not raced");
        return;
    }
    s_gdma_ch = ch;
    if (desc == nullptr || count == 0 || count % rows != 0) {
        ESP_LOGW(TAG, "scan: layout not modelled");
        return;
    }
    const uint32_t per_row = count / rows;
    const uint32_t first = desc[0].dw0.length;
    const uint32_t rest = per_row > 1 ? desc[1].dw0.length : 0;
    for (uint32_t i = 0; i < count; i++) {
        const uint32_t want = (i % per_row) == 0 ? first : rest;
        if (desc[i].dw0.length != want) {
            ESP_LOGW(TAG, "scan: layout not modelled");
            return;
        }
    }
    s_scan_count = count;
    s_scan_per_row = per_row;
    s_scan_first = first;
    s_scan_rest = rest;
    s_scan_row_bytes = first + (per_row - 1) * rest;
    s_scan_desc = reinterpret_cast<uint32_t>(desc);
    if (s_pins_check) {
        s3_pins_check(per_row);
    }
    // As the classic's: four wraps, the shortest of the three intervals,
    // held to 0.67-2x the library's computed rate.
    int64_t wraps[4] = {0, 0, 0, 0};
    int seen = 0;
    uint32_t last = scan_desc_index();
    const int64_t give_up = esp_timer_get_time() + 150000;
    while (seen < 4 && esp_timer_get_time() < give_up) {
        const uint32_t idx = scan_desc_index();
        if (idx != UINT32_MAX && last != UINT32_MAX && idx + count / 2 < last) {
            wraps[seen++] = esp_timer_get_time();
        }
        if (idx != UINT32_MAX) {
            last = idx;
        }
    }
    if (seen < 4) {
        ESP_LOGW(TAG, "scan: no refresh in 150 ms (%s, GDMA channel %d)", S3_SRC_NAME[s_scan_src], ch);
        s_scan_desc = 0;
        return;
    }
    uint32_t period = UINT32_MAX;
    for (int i = 1; i < 4; i++) {
        const uint32_t d = static_cast<uint32_t>(wraps[i] - wraps[i - 1]);
        period = d < period ? d : period;
    }
    const int calc = s_panel->calculated_refresh_rate;
    const uint32_t expect = calc > 0 ? 1000000u / static_cast<uint32_t>(calc) : 0;
    if (expect == 0 || period * 3 < expect * 2 || period > expect * 2) {
        ESP_LOGW(TAG, "scan: refresh %u us measured, %u us expected: implausible, not used",
                 (unsigned)period, (unsigned)expect);
        s_scan_desc = 0;
        return;
    }
    s_scan_row_us = period / rows;
    ESP_LOGI(TAG, "scan: %u descriptors, %u per row pair, refresh %u us (GDMA channel %d, racing on %s)",
             (unsigned)count, (unsigned)per_row, (unsigned)period, ch, S3_SRC_NAME[s_scan_src]);
}
#endif /* CONFIG_IDF_TARGET_ESP32S3 */

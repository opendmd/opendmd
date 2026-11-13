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
#include "soc/lcd_cam_struct.h"
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
    /* The S3's LCD_CAM is fed by a GDMA channel, whose own register holds
     * the descriptor it is on (GDMA's OUT_DSCR): not read yet (S3 port
     * phase 1/2). hub75_scan_init never models the scan there, so this is
     * not reached; the beam is not raced and panel_sync has no scan. */
    return UINT32_MAX;
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
    ESP_LOGW(TAG, "scan: not modelled on the ESP32-S3 yet (GDMA): the beam is not raced");
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

extern "C" bool hub75_init(const hub75_cfg_t *cfg) {
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

extern "C" void hub75_clear(void) {
    if (s_panel != nullptr) {
        s_panel->clearScreen();
    }
}

extern "C" void hub75_draw_rgb888(const uint8_t *rgb, uint16_t w, uint16_t h) {
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

extern "C" void hub75_set_pixel(uint16_t x, uint16_t y, uint8_t r, uint8_t g, uint8_t b) {
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
extern "C" IRAM_ATTR void hub75_draw_span(const uint8_t *rgb_row, uint16_t y, uint16_t x0, uint16_t x1) {
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
extern "C" IRAM_ATTR void hub75_draw_row_pair(const uint8_t *top_row, const uint8_t *bottom_row, uint16_t y) {
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

extern "C" void hub75_flip(void) {
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

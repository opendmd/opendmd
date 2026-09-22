// C++ side of the pure-C facade declared in include/hub75_shim.h.
//
// Everything C++ about mrcodetastic/ESP32-HUB75-MatrixPanel-I2S-DMA (the
// class, the config struct, its enums) is confined here so that bindgen only
// ever sees the C header.

#include "hub75_shim.h"

#include <string.h>

#include "ESP32-HUB75-MatrixPanel-I2S-DMA.h"
#include "esp_log.h"

static const char *TAG = "hub75_shim";

static MatrixPanel_I2S_DMA *s_panel = nullptr;
static uint16_t s_width = 0;
static uint16_t s_height = 0;

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

    s_width = static_cast<uint16_t>(cfg->mx_width * cfg->chain_length);
    s_height = cfg->mx_height;
    s_panel->clearScreen();
    ESP_LOGI(TAG, "panel %ux%u up, refresh %d Hz", s_width, s_height,
             s_panel->calculated_refresh_rate);
    return true;
}

extern "C" void hub75_set_brightness(uint8_t brightness) {
    if (s_panel != nullptr) {
        s_panel->setBrightness8(brightness);
    }
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

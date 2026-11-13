/*
 * hub75_shim.cpp's ESP32-S3 part (included there under
 * CONFIG_IDF_TARGET_ESP32S3 only; the classic ESP32 never sees it). The
 * definitions are at the end of hub75_shim.cpp, after the statics they use;
 * this declares them for the S3 branches above.
 *
 * Why an include: the classic build must stay byte-identical (its ELF's
 * SHA-256 is in the app image, and the ELF carries line numbers), so the S3
 * code takes no line of hub75_shim.cpp away from the classic's.
 */
#include "sdkconfig.h"
#include "soc/lcd_cam_struct.h"
#include "soc/gdma_struct.h"
#include "soc/gpio_reg.h"
#include "esp_private/gdma.h"
#include "esp_rom_sys.h"
#include "esp_heap_caps.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

/* The register hub75_scan_pos reads (hub75_scan_set_source). */
#define HUB75_S3_SRC_DSCR 0
#define HUB75_S3_SRC_BF0 1
#define HUB75_S3_SRC_BF1 2
#define HUB75_S3_SRC_STATE 3

/* The descriptor the GDMA channel's chosen register names, as an index into
 * the block (UINT32_MAX outside it). */
static uint32_t s3_scan_desc_index(void);
/* hub75_scan_init's body on the S3. */
static void s3_scan_init(void);
/* hub75_init's first step on the S3: *out a copy of *cfg to init with
 * (double_buff as hub75_set_buffers and the heap decide). True: hub75_init
 * returns *result at once (power-safe: no panel started, every HUB75 pin an
 * input; or the double-buffered init done by a nested hub75_init). */
static bool s3_init_hook(const hub75_cfg_t *cfg, hub75_cfg_t *out, bool *result);
/* The writers' first step (hub75_clear, _draw_rgb888, _set_pixel,
 * _draw_span, _draw_row_pair): double-buffered, wait for the last flip's
 * switch (and copy across) before the back buffer is written; note the row
 * pair written (0xFFFF: all). */
static void s3_before_write(uint32_t row);
/* hub75_flip's first step: true = done here (double-buffered). */
static bool s3_flip(void);

/* At the top of hub75_init (the classic's is empty: hub75_shim.h). */
#undef HUB75_S3_INIT_HOOK
#define HUB75_S3_INIT_HOOK hub75_cfg_t s3_cfg_; { bool s3_r_; if (s3_init_hook(cfg, &s3_cfg_, &s3_r_)) { return s3_r_; } } cfg = &s3_cfg_;
#undef HUB75_S3_WRITE_HOOK
#define HUB75_S3_WRITE_HOOK(row) s3_before_write(static_cast<uint32_t>(row));
#undef HUB75_S3_FLIP_HOOK
#define HUB75_S3_FLIP_HOOK if (s3_flip()) { return; }

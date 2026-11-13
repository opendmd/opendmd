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
#include "soc/lcd_cam_struct.h"
#include "soc/gdma_struct.h"
#include "soc/gpio_reg.h"
#include "esp_private/gdma.h"
#include "esp_rom_sys.h"

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
/* hub75_init's first step on the S3: true = power-safe (the panel is not
 * started: OE driven high, every other HUB75 pin left as the reset left it). */
static bool s3_init_hook(const hub75_cfg_t *cfg);

/* At the top of hub75_init (the classic's is empty: hub75_shim.h). */
#undef HUB75_S3_INIT_HOOK
#define HUB75_S3_INIT_HOOK if (s3_init_hook(cfg)) { return true; }

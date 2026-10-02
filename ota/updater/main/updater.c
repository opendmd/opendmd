/*
 * The OpenDMD updater (docs/ota.md, steps 3 and 4).
 *
 * Lives in the `ota_0` partition (0x260000, 384 KB). The firmware boots it
 * once, after it has received a firmware bundle over the LAN, verified its
 * Ed25519 signature, written it to the card as /update/opendmd.bin and
 * pointed otadata at ota_0. This program then:
 *
 *   1. blanks the panel (OE high, every other HUB75 line low) as early as
 *      the app can: the same CORE-stage hook as the firmware's hub75_shim;
 *   2. mounts the card on SPI3 (SCK 18, MISO 19, MOSI 23, CS 5) at 20 MHz;
 *   3. reads the 140-byte bundle header, checks the magic, that the image
 *      fits the factory partition, that the file holds exactly the image,
 *      and hashes the whole image off the card *before* erasing anything
 *      (SHA-256, mbedtls), with the ESP image magic 0xE9 at its start;
 *   4. erases the factory partition a 64 KB block ahead of the writes and
 *      writes the image in 4 KB pieces, hashing again as it goes (the card
 *      is read twice: what was written is what was hashed);
 *   5. esp_image_verify() on the result, esp_ota_set_boot_partition(factory)
 *      (which erases otadata: the bootloader boots factory from then on),
 *      renames the file to /update/done-<version>.bin, appends to
 *      /update/log.txt, restarts into the new firmware.
 *
 * Failure (step 4 of the design): nothing written yet (no card, no file,
 * a bad file) and the factory image verifies -> boot it. Otherwise stay,
 * blink fast and try again every 10 s (the card may be put back). The
 * signature is not checked here (no Ed25519 in the updater): the firmware
 * never points otadata at ota_0 without a verified file on the card, and
 * the SHA-256 in the header ties the file to what was verified.
 *
 * Every step is logged on the console (921600 baud, as the firmware) and,
 * once the card is mounted, to /update/log.txt.
 */

#include <dirent.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "driver/gpio.h"
#include "driver/sdspi_host.h"
#include "driver/spi_common.h"
#include "esp_err.h"
#include "esp_image_format.h"
#include "esp_log.h"
#include "esp_ota_ops.h"
#include "esp_partition.h"
#include "esp_private/startup_internal.h"
#include "esp_rom_gpio.h"
#include "esp_system.h"
#include "esp_timer.h"
#include "esp_vfs_fat.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "hal/gpio_ll.h"
#include "mbedtls/sha256.h"
#include "sdmmc_cmd.h"
#include "soc/gpio_sig_map.h"
#include "soc/soc_caps.h"

static const char *TAG = "updater";

/* --- Pins: keep in step with firmware/src/config.rs ---------------------- */

/* HUB75 output enable: high = panel off. */
#define PIN_OE 15
/* R1 G1 B1 R2 G2 B2, A B C D, LAT, CLK: driven low. */
static const uint8_t HUB75_LOW[] = {25, 26, 27, 14, 12, 13, 33, 32, 22, 17, 4, 16};
#define PIN_LED 2
#define PIN_SD_SCK 18
#define PIN_SD_MISO 19
#define PIN_SD_MOSI 23
#define PIN_SD_CS 5
#define SD_KHZ 20000

/* --- The bundle (opendmd-core/src/ota.rs) -------------------------------- */

#define MOUNT "/sdcard"
#define UPDATE_DIR MOUNT "/update"
#define BUNDLE_PATH UPDATE_DIR "/opendmd.bin"
#define LOG_PATH UPDATE_DIR "/log.txt"
static const uint8_t MAGIC[8] = {'O', 'D', 'F', 'W', '1', 0, 0, 0};
#define VERSION_LEN 32
#define HEADER_LEN 140
#define ESP_IMAGE_MAGIC 0xE9

#define PIECE 4096
#define BLOCK (64 * 1024)
#define RETRY_MS 10000
#define BLINK_SLOW_MS 500
#define BLINK_FAST_MS 100

/* --- The panel, blanked before anything else ------------------------------ */

/*
 * One pin driven as a plain GPIO, register writes only (no locks, no
 * driver): GPIO_OUT's bit first, output enable, the matrix output back from
 * whatever peripheral had it (I2S1, after a CPU-only reset out of the
 * firmware) to GPIO_OUT, then the pad's IO_MUX function. As hub75_shim's
 * oe_high(), for every HUB75 line.
 */
static void drive(uint32_t pin, uint32_t level)
{
    if (pin >= SOC_GPIO_PIN_COUNT || ((SOC_GPIO_VALID_OUTPUT_GPIO_MASK >> pin) & 1U) == 0) {
        return;
    }
    gpio_ll_set_level(&GPIO, pin, level);
    gpio_ll_output_enable(&GPIO, pin);
    esp_rom_gpio_connect_out_signal(pin, SIG_GPIO_OUT_IDX, false, false);
    esp_rom_gpio_pad_select_gpio(pin);
}

/*
 * The earliest the app can do it: the CORE start-up stage, priority 2,
 * on CPU0 (hub75_shim's hub75_early_blank). The firmware's restart already
 * stopped the I2S DMA and drove OE high; this keeps it so, and makes the
 * data lines quiet, from before the heap exists.
 */
ESP_SYSTEM_INIT_FN(updater_early_blank, CORE, BIT(0), 2)
{
    drive(PIN_OE, 1);
    for (size_t i = 0; i < sizeof HUB75_LOW; i++) {
        drive(HUB75_LOW[i], 0);
    }
    return ESP_OK;
}

/* --- The LED ---------------------------------------------------------------- */

/* Half-period in ms; 0 = off. */
static volatile int s_blink_ms;

static void led_task(void *arg)
{
    (void)arg;
    int on = 0;
    for (;;) {
        int ms = s_blink_ms;
        if (ms <= 0) {
            gpio_set_level(PIN_LED, 0);
            on = 0;
            vTaskDelay(pdMS_TO_TICKS(50));
            continue;
        }
        on = !on;
        gpio_set_level(PIN_LED, on);
        vTaskDelay(pdMS_TO_TICKS(ms));
    }
}

/* --- Logging: the console always, the card's log.txt once mounted --------- */

static bool s_mounted;
static sdmmc_card_t *s_card;
/* Lines from before the mount, written to log.txt when it happens. */
static char s_backlog[1024];
static size_t s_backlog_len;

static void note(const char *fmt, ...)
{
    char line[200];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(line, sizeof line, fmt, ap);
    va_end(ap);
    ESP_LOGI(TAG, "%s", line);
    char stamped[224];
    int n = snprintf(stamped, sizeof stamped, "updater +%.1f s: %s\n",
                     esp_timer_get_time() / 1e6, line);
    if (n <= 0) {
        return;
    }
    if (!s_mounted) {
        if (s_backlog_len + (size_t)n < sizeof s_backlog) {
            memcpy(s_backlog + s_backlog_len, stamped, (size_t)n);
            s_backlog_len += (size_t)n;
        }
        return;
    }
    FILE *f = fopen(LOG_PATH, "a");
    if (f != NULL) {
        fputs(stamped, f);
        fclose(f);
    }
}

/* --- The card ----------------------------------------------------------------- */

static esp_err_t mount(void)
{
    static bool bus_ready;
    if (s_mounted) {
        return ESP_OK;
    }
    if (!bus_ready) {
        spi_bus_config_t bus = {
            .mosi_io_num = PIN_SD_MOSI,
            .miso_io_num = PIN_SD_MISO,
            .sclk_io_num = PIN_SD_SCK,
            .quadwp_io_num = -1,
            .quadhd_io_num = -1,
            .max_transfer_sz = PIECE,
        };
        esp_err_t err = spi_bus_initialize(SPI3_HOST, &bus, SDSPI_DEFAULT_DMA);
        if (err != ESP_OK) {
            note("SPI3 bus init failed: %s", esp_err_to_name(err));
            return err;
        }
        gpio_set_pull_mode(PIN_SD_MISO, GPIO_PULLUP_ONLY);
        gpio_set_pull_mode(PIN_SD_CS, GPIO_PULLUP_ONLY);
        bus_ready = true;
    }
    sdmmc_host_t host = SDSPI_HOST_DEFAULT();
    host.slot = SPI3_HOST;
    host.max_freq_khz = SD_KHZ;
    sdspi_device_config_t slot = SDSPI_DEVICE_CONFIG_DEFAULT();
    slot.gpio_cs = PIN_SD_CS;
    slot.host_id = SPI3_HOST;
    esp_vfs_fat_sdmmc_mount_config_t cfg = {
        .format_if_mount_failed = false,
        .max_files = 3,
        .allocation_unit_size = 16 * 1024,
    };
    esp_err_t err = esp_vfs_fat_sdspi_mount(MOUNT, &host, &slot, &cfg, &s_card);
    if (err != ESP_OK) {
        note("card mount failed: %s", esp_err_to_name(err));
        return err;
    }
    s_mounted = true;
    note("card mounted at %d kHz", SD_KHZ);
    if (s_backlog_len > 0) {
        FILE *f = fopen(LOG_PATH, "a");
        if (f != NULL) {
            fwrite(s_backlog, 1, s_backlog_len, f);
            fclose(f);
        }
        s_backlog_len = 0;
    }
    return ESP_OK;
}

static void unmount(void)
{
    if (s_mounted) {
        esp_vfs_fat_sdcard_unmount(MOUNT, s_card);
        s_mounted = false;
        s_card = NULL;
    }
}

/* Read exactly `n` bytes, or fail. */
static bool read_exact(FILE *f, uint8_t *buf, size_t n)
{
    size_t have = 0;
    while (have < n) {
        size_t r = fread(buf + have, 1, n - have, f);
        if (r == 0) {
            return false;
        }
        have += r;
    }
    return true;
}

/* --- The factory partition ---------------------------------------------------- */

static const esp_partition_t *factory(void)
{
    return esp_partition_find_first(ESP_PARTITION_TYPE_APP, ESP_PARTITION_SUBTYPE_APP_FACTORY, NULL);
}

static bool factory_verifies(void)
{
    const esp_partition_t *p = factory();
    if (p == NULL) {
        return false;
    }
    esp_partition_pos_t pos = {.offset = p->address, .size = p->size};
    esp_image_metadata_t meta;
    return esp_image_verify(ESP_IMAGE_VERIFY_SILENT, &pos, &meta) == ESP_OK;
}

static void boot_factory(void)
{
    const esp_partition_t *p = factory();
    esp_err_t err = p != NULL ? esp_ota_set_boot_partition(p) : ESP_ERR_NOT_FOUND;
    if (err != ESP_OK) {
        /* otadata still says ota_0: the next reset comes back here, which
         * retries; nothing better to do. */
        note("setting the boot partition to factory failed: %s", esp_err_to_name(err));
    }
    note("restarting into the firmware");
    unmount();
    s_blink_ms = 0;
    vTaskDelay(pdMS_TO_TICKS(100));
    esp_restart();
}

/* --- One attempt ------------------------------------------------------------- */

typedef enum {
    /* The new firmware is in place and set to boot. */
    R_DONE,
    /* Nothing was written to the factory partition. */
    R_UNTOUCHED,
    /* The factory partition was (partly) overwritten and is not good. */
    R_WRITTEN_BAD,
} outcome_t;

static uint8_t s_buf[PIECE];

/* Make the version safe for a file name. */
static void sanitize(char *s)
{
    for (; *s; s++) {
        char c = *s;
        bool ok = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') ||
                  c == '.' || c == '-' || c == '+' || c == '_';
        if (!ok) {
            *s = '_';
        }
    }
}

/* SHA-256 of the next `len` bytes of `f`, first byte in *first. */
static bool hash_file(FILE *f, uint32_t len, uint8_t out[32], uint8_t *first)
{
    mbedtls_sha256_context sha;
    mbedtls_sha256_init(&sha);
    mbedtls_sha256_starts(&sha, 0);
    bool ok = true;
    for (uint32_t off = 0; off < len;) {
        size_t n = len - off < PIECE ? len - off : PIECE;
        if (!read_exact(f, s_buf, n)) {
            note("reading the card failed at %u of %u B", (unsigned)off, (unsigned)len);
            ok = false;
            break;
        }
        if (off == 0) {
            *first = s_buf[0];
        }
        mbedtls_sha256_update(&sha, s_buf, n);
        off += n;
        if ((off & (BLOCK - 1)) == 0) {
            vTaskDelay(1);
        }
    }
    if (ok) {
        mbedtls_sha256_finish(&sha, out);
    }
    mbedtls_sha256_free(&sha);
    return ok;
}

static outcome_t attempt(void)
{
    const esp_partition_t *part = factory();
    if (part == NULL) {
        note("no factory partition in this table");
        return R_UNTOUCHED;
    }
    if (mount() != ESP_OK) {
        return R_UNTOUCHED;
    }
    FILE *f = fopen(BUNDLE_PATH, "rb");
    if (f == NULL) {
        note("no %s on the card", BUNDLE_PATH + sizeof MOUNT - 1);
        return R_UNTOUCHED;
    }
    struct stat st;
    uint8_t head[HEADER_LEN];
    if (fstat(fileno(f), &st) != 0 || !read_exact(f, head, HEADER_LEN)) {
        note("cannot read the bundle's header");
        fclose(f);
        return R_UNTOUCHED;
    }
    if (memcmp(head, MAGIC, sizeof MAGIC) != 0) {
        note("not a firmware bundle (bad magic)");
        fclose(f);
        return R_UNTOUCHED;
    }
    char version[VERSION_LEN + 1];
    memcpy(version, head + 8, VERSION_LEN);
    version[VERSION_LEN] = 0;
    sanitize(version);
    uint32_t len = (uint32_t)head[40] | (uint32_t)head[41] << 8 | (uint32_t)head[42] << 16 |
                   (uint32_t)head[43] << 24;
    const uint8_t *want = head + 44;
    if (len == 0 || len > part->size) {
        note("the image is %u B, the factory partition is %u B", (unsigned)len, (unsigned)part->size);
        fclose(f);
        return R_UNTOUCHED;
    }
    if ((uint64_t)st.st_size != (uint64_t)HEADER_LEN + len) {
        note("the file is %ld B, the bundle says %u B", (long)st.st_size, (unsigned)(HEADER_LEN + len));
        fclose(f);
        return R_UNTOUCHED;
    }
    note("bundle %s, image %u B: checking it on the card", version, (unsigned)len);

    /* Pass 1: the file, before the factory partition is touched. */
    uint8_t got[32];
    uint8_t first = 0;
    int64_t t0 = esp_timer_get_time();
    if (!hash_file(f, len, got, &first)) {
        fclose(f);
        return R_UNTOUCHED;
    }
    if (first != ESP_IMAGE_MAGIC) {
        note("not an ESP32 app image (first byte 0x%02X)", first);
        fclose(f);
        return R_UNTOUCHED;
    }
    if (memcmp(got, want, 32) != 0) {
        note("SHA-256 mismatch on the card: the file is damaged");
        fclose(f);
        return R_UNTOUCHED;
    }
    note("SHA-256 ok (%.1f s); writing the firmware partition", (esp_timer_get_time() - t0) / 1e6);

    /* Pass 2: erase a block ahead, write in pieces, hash what is written. */
    if (fseek(f, HEADER_LEN, SEEK_SET) != 0) {
        note("seek failed");
        fclose(f);
        return R_UNTOUCHED;
    }
    s_blink_ms = BLINK_SLOW_MS;
    t0 = esp_timer_get_time();
    mbedtls_sha256_context sha;
    mbedtls_sha256_init(&sha);
    mbedtls_sha256_starts(&sha, 0);
    outcome_t result = R_DONE;
    for (uint32_t off = 0; off < len;) {
        if ((off & (BLOCK - 1)) == 0) {
            size_t span = part->size - off < BLOCK ? part->size - off : BLOCK;
            esp_err_t err = esp_partition_erase_range(part, off, span);
            if (err != ESP_OK) {
                note("erase at 0x%x failed: %s", (unsigned)off, esp_err_to_name(err));
                result = R_WRITTEN_BAD;
                break;
            }
            vTaskDelay(1);
        }
        size_t n = len - off < PIECE ? len - off : PIECE;
        if (!read_exact(f, s_buf, n)) {
            note("reading the card failed at %u of %u B, mid-write", (unsigned)off, (unsigned)len);
            result = R_WRITTEN_BAD;
            break;
        }
        mbedtls_sha256_update(&sha, s_buf, n);
        esp_err_t err = esp_partition_write(part, off, s_buf, n);
        if (err != ESP_OK) {
            note("write at 0x%x failed: %s", (unsigned)off, esp_err_to_name(err));
            result = R_WRITTEN_BAD;
            break;
        }
        off += n;
        if ((off & (256 * 1024 - 1)) == 0) {
            ESP_LOGI(TAG, "written %u of %u B", (unsigned)off, (unsigned)len);
        }
        vTaskDelay(1);
    }
    fclose(f);
    if (result == R_DONE) {
        mbedtls_sha256_finish(&sha, got);
    }
    mbedtls_sha256_free(&sha);
    if (result != R_DONE) {
        return result;
    }
    if (memcmp(got, want, 32) != 0) {
        note("SHA-256 mismatch on the second read: the card is unreliable");
        return R_WRITTEN_BAD;
    }
    note("written in %.1f s; verifying the image", (esp_timer_get_time() - t0) / 1e6);

    esp_partition_pos_t pos = {.offset = part->address, .size = part->size};
    esp_image_metadata_t meta;
    esp_err_t err = esp_image_verify(ESP_IMAGE_VERIFY, &pos, &meta);
    if (err != ESP_OK) {
        note("esp_image_verify failed: %s", esp_err_to_name(err));
        return R_WRITTEN_BAD;
    }
    err = esp_ota_set_boot_partition(part);
    if (err != ESP_OK) {
        note("setting the boot partition to factory failed: %s", esp_err_to_name(err));
        return R_WRITTEN_BAD;
    }
    char done[64 + VERSION_LEN];
    snprintf(done, sizeof done, UPDATE_DIR "/done-%s.bin", version);
    unlink(done);
    if (rename(BUNDLE_PATH, done) != 0) {
        note("renaming the bundle to %s failed (the firmware is updated anyway)", done + sizeof MOUNT - 1);
    }
    note("updated to %s (%u B, SHA-256 ok, image verified)", version, (unsigned)len);
    return R_DONE;
}

void app_main(void)
{
    /* Again through the driver, which also takes GPIO32/33 off the RTC mux. */
    gpio_set_direction(PIN_OE, GPIO_MODE_OUTPUT);
    gpio_set_level(PIN_OE, 1);
    for (size_t i = 0; i < sizeof HUB75_LOW; i++) {
        gpio_set_direction(HUB75_LOW[i], GPIO_MODE_OUTPUT);
        gpio_set_level(HUB75_LOW[i], 0);
    }
    gpio_reset_pin(PIN_LED);
    gpio_set_direction(PIN_LED, GPIO_MODE_OUTPUT);
    xTaskCreate(led_task, "led", 2048, NULL, 2, NULL);

    const esp_app_desc_t *me = esp_app_get_description();
    note("OpenDMD %s started (panel off)", me->version);

    for (int n = 1;; n++) {
        s_blink_ms = 0;
        outcome_t r = attempt();
        if (r == R_DONE) {
            boot_factory();
        }
        if (r == R_UNTOUCHED && factory_verifies()) {
            note("attempt %d: nothing written and the firmware there verifies: booting it", n);
            boot_factory();
        }
        s_blink_ms = BLINK_FAST_MS;
        note("attempt %d failed (%s); the firmware partition does not verify: retrying in %d s",
             n, r == R_WRITTEN_BAD ? "partly written" : "nothing written", RETRY_MS / 1000);
        unmount();
        vTaskDelay(pdMS_TO_TICKS(RETRY_MS));
    }
}

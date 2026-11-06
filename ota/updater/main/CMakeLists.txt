idf_component_register(SRCS "updater.c" "panel.cpp" "panel_timing.c"
                       INCLUDE_DIRS "."
                       PRIV_REQUIRES fatfs sdmmc esp_driver_sdspi esp_driver_spi esp_driver_gpio
                                     esp_partition app_update bootloader_support mbedtls
                                     esp_system esp_rom esp_hw_support esp_timer log spi_flash esp_app_format
                                     nvs_flash ESP32-HUB75-MatrixPanel-I2S-DMA)

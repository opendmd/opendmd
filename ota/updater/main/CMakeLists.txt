idf_component_register(SRCS "updater.c"
                       INCLUDE_DIRS "."
                       PRIV_REQUIRES fatfs sdmmc esp_driver_sdspi esp_driver_spi esp_driver_gpio
                                     esp_partition app_update bootloader_support mbedtls
                                     esp_system esp_rom esp_hw_support esp_timer log)

# netdiag.c: the emulator's walk of lwIP's held frames (emu-census builds
# only call it; the linker drops it from the rest); netstat.c: `/api/status`
# `net`'s counts and the abort of a dead connection at close. Hence lwip below.
idf_component_register(SRCS "hub75_shim.cpp" "netdiag.c" "netstat.c"
                       INCLUDE_DIRS "include"
                       REQUIRES ESP32-HUB75-MatrixPanel-I2S-DMA esp_timer lwip)

# The project builds C components at -Os (CONFIG_COMPILER_OPTIMIZATION_SIZE).
# hub75_draw_span is the per-frame hot loop, so this one file is optimised.
target_compile_options(${COMPONENT_LIB} PRIVATE -O2)

# The SPI master driver stays at -Og. With all of IDF at -Os
# (CONFIG_COMPILER_OPTIMIZATION_SIZE, for the OTA slot), SD reads timed out
# (sdmmc_send_cmd 0x107, then every read failing until a reboot) whenever the
# httpd served a big file while the player read the card: 27 of 30
# back-to-back GETs of /filelist.txt failed on a test panel. Bisected by
# component: sdmmc + esp_driver_sdspi at -Og still failed (56 of 60), and
# esp_driver_spi alone at -Og passed 60 of 60, as did -Og everywhere (160 or
# 240 MHz). This component is just a convenient place in the project to
# reach that library from; the last -O on the command line wins.
foreach(sd_component esp_driver_spi)
  idf_component_get_property(sd_lib ${sd_component} COMPONENT_LIB)
  target_compile_options(${sd_lib} PRIVATE -Og)
endforeach()

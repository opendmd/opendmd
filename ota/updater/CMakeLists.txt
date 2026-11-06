# The OpenDMD updater (docs/ota.md): a small ESP-IDF app in the `ota_0`
# partition that copies a staged, verified firmware image from the card
# into the `factory` partition. Built with idf.py: `make updater` (build
# dir UPDATER_BUILD, by default target/updater).
cmake_minimum_required(VERSION 3.16)
# The HUB75 driver the firmware uses (the submodule: `git submodule update
# --init`), for the picture on the panel while it installs (main/panel.cpp).
set(EXTRA_COMPONENT_DIRS "${CMAKE_CURRENT_LIST_DIR}/../../firmware/components/ESP32-HUB75-MatrixPanel-I2S-DMA")
include($ENV{IDF_PATH}/tools/cmake/project.cmake)
project(opendmd-updater)

# The OpenDMD updater (docs/ota.md): a small ESP-IDF app in the `ota_0`
# partition that copies a staged, verified firmware image from the card
# into the `factory` partition. Built with idf.py: `make updater` (build
# dir UPDATER_BUILD, by default target/updater).
cmake_minimum_required(VERSION 3.16)
include($ENV{IDF_PATH}/tools/cmake/project.cmake)
project(opendmd-updater)

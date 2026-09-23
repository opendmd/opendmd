idf_component_register(SRCS "hub75_shim.cpp"
                       INCLUDE_DIRS "include"
                       REQUIRES ESP32-HUB75-MatrixPanel-I2S-DMA)

# The project builds C components at -Og (CONFIG_COMPILER_OPTIMIZATION_DEBUG).
# hub75_draw_span is the per-frame hot loop, so this one file is optimised.
target_compile_options(${COMPONENT_LIB} PRIVATE -O2)

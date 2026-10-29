# The smallest kernel: proves a block of executable heap runs (phase 0 of
# docs/research/psi-jit-esp32.md). Windowed ABI: `extern "C" fn() -> i32`.
	.text
	.align	4
	.global	tiny_kernel
tiny_kernel:
	entry	a1, 32
	movi.n	a2, 42
	retw.n

# A kernel calling a Rust helper through the windowed ABI (callx8): the
# path a real generator takes for `len`, `noise`, `atan2`.
# `extern "C" fn(f: extern "C" fn(i32) -> i32, x: i32) -> i32`: f(x).
	.text
	.align	4
	.global	call_kernel
call_kernel:
	entry	a1, 32
	mov.n	a10, a3
	callx8	a2
	mov.n	a2, a10
	retw.n

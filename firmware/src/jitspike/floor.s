# Rotozoom floor's narrowed pixel stage, by hand, as a JIT would lay it out
# (the listing of 2026-10-29; see opendmd-notes docs/research/psi-jit-esp32.md, phase 0):
#   L1 = mul_w(s34, L0); L3 = add_w(L1, s36)
#   L1 = mul_w(s35, L0); L4 = add_w(L1, L2); L1 = add_w(L4, s15)
#   L4 = floor(L3); L3 = floor(L1); L1 = add_w(L4, L3); L4 = mask(L1, 2 - 2^-16)
#   L1 = select(L4, 0.78, 0.28); out = pal(fire, L1) * s23
# L0 (dx) and L2 (col) are the column run's lanes; the two colours are the
# row's (worked out once a row, as the VM's fused `pal_select` does once a
# lane). `extern "C" fn(dx: *const i32, col: *const i32, out: *mut u8,
# params: *const i32)`; params: [0] s34, [4] s36, [8] s35, [12] s15,
# [16..19] the colour when the cell is 1 (r g b), [19..22] when 0.
	.text
	.align	4
	.global	floor_kernel
floor_kernel:
	entry	a1, 32
	l32i	a8, a5, 0
	l32i	a9, a5, 4
	l32i	a10, a5, 8
	l32i	a11, a5, 12
	movi	a7, -1
	slli	a7, a7, 16
	ssai	16
	addmi	a6, a2, 512
1:
	l32i	a12, a2, 0
	l32i	a13, a3, 0
	mull	a14, a8, a12
	mulsh	a15, a8, a12
	src	a14, a15, a14
	add	a14, a14, a9
	mull	a15, a10, a12
	mulsh	a12, a10, a12
	src	a15, a12, a15
	add	a15, a15, a13
	add	a15, a15, a11
	and	a14, a14, a7
	and	a15, a15, a7
	add	a14, a14, a15
	extui	a14, a14, 16, 1
	addx2	a15, a14, a14
	sub	a15, a5, a15
	l8ui	a12, a15, 19
	l8ui	a13, a15, 20
	l8ui	a14, a15, 21
	s8i	a12, a4, 0
	s8i	a13, a4, 1
	s8i	a14, a4, 2
	addi	a2, a2, 4
	addi	a3, a3, 4
	addi	a4, a4, 3
	bne	a2, a6, 1b
	retw

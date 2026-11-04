-- What the gallery stores of a Formula's compiled form (the bytes the panel
-- plays, a `@formula?psb=…` link carries): the gallery's size rule is the
-- compiled length now (opendmd-gallery MAX_PSB, 4,500 B), not the source's,
-- and the tile says which firmware a Formula needs when it is newer than the
-- oldest release the site serves (the compiled format's version, `psi_abi`).
-- Additive only: null on a row stored before this migration (its tile says
-- nothing about either). The examples are seeded again at each deploy; a
-- contribution is checked again when its file changes, or when its `sha` is
-- cleared (update gallery set sha = '' where source = 'gallery'; the next
-- sync takes PER_RUN of them at a time).
alter table gallery add column psb_len integer;
alter table gallery add column abi     integer;
-- (the speed-up's estimate beside esp32_us, for the tile's "about N ms a frame")
alter table gallery add column jit_us  integer;

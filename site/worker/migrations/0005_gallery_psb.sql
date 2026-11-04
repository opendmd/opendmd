-- What the gallery stores of a Formula's compiled form (the bytes the panel
-- plays, a `@formula?psb=…` link carries): the gallery's size rule is the
-- compiled length now (opendmd-gallery MAX_PSB, 4,500 B), not the source's,
-- and the tile says which firmware a Formula needs when its compiled format
-- (`psi_abi`) is newer than the one the latest release plays.
-- Additive only: null on a row stored before this migration (its tile says
-- nothing about either). The examples are seeded again at each deploy; a
-- contribution is checked again when its file changes, or when its `sha` is
-- cleared (update gallery set sha = '' where source = 'gallery'; the next
-- sync takes PER_RUN of them at a time).
alter table gallery add column psb_len integer;
alter table gallery add column abi     integer;
-- esp32_us is the tier figure from now on (the panel's JIT estimate where the
-- JIT takes the Formula, else the VM's; opendmd-gallery `Program::tier_us`,
-- what the 10 ms limit counts); vm_us is the VM's, "without the JIT", shown
-- in the tile's hover text. Null on a row stored before this migration.
alter table gallery add column vm_us   integer;

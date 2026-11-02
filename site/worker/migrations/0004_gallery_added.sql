-- When each gallery entry was added (unix seconds), so /formulas/ can list
-- the newest first and sort and filter (tools/gallery_page.py `order`,
-- src/gallery.ts `lists`). OpenDMD's examples carry theirs from the build's
-- formulas/gallery.json (opendmd-web/formulas/added.json), set at each
-- seed; a contribution's is when the Worker first accepted it (the sync
-- has the files, not their history), kept through later versions. A
-- contribution stored before this has its last change as its date.
alter table gallery add column added integer;
update gallery set added = updated where source = 'gallery' and added is null;

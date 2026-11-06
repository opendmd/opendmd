-- Anonymous short links are gone (they risked making
-- opendmd.dev a pastebin; sharing is submitting to the gallery, reviewed on
-- GitHub). Their table goes; the gallery store (0002) shares this database
-- and stays. Applied by make site-d1-migrate, which make site-deploy-built
-- runs before it deploys.
drop index if exists formulas_created;
drop table if exists formulas;

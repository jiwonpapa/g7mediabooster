ALTER TABLE jobs ADD COLUMN site_policy_revision INTEGER;

CREATE INDEX jobs_site_policy_revision_idx
    ON jobs (site_policy_revision)
    WHERE site_policy_revision IS NOT NULL;

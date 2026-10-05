-- Retain settled financial history while removing a workspace from active use.
ALTER TABLE organizations ADD COLUMN archived_at TIMESTAMPTZ;
CREATE INDEX organizations_active_owner ON organizations(owner_id) WHERE archived_at IS NULL;

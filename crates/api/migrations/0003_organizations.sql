CREATE TABLE organizations (
    id UUID PRIMARY KEY,
    owner_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 2 AND 100),
    version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
    creation_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX organizations_owner ON organizations(owner_id);
CREATE TABLE organization_members (
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('ADMIN','MEMBER')),
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (organization_id,user_id)
);
CREATE INDEX organization_members_user ON organization_members(user_id);
CREATE TABLE company_programs (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    title TEXT NOT NULL CHECK (char_length(title) BETWEEN 2 AND 100),
    target_m INTEGER NOT NULL CHECK (target_m BETWEEN 1000 AND 5000),
    currency TEXT NOT NULL CHECK (currency = 'CZK'),
    reward_minor BIGINT NOT NULL CHECK (reward_minor BETWEEN 100 AND 1000000),
    max_participants INTEGER NOT NULL CHECK (max_participants BETWEEN 1 AND 10000),
    state TEXT NOT NULL DEFAULT 'DRAFT' CHECK (state IN ('DRAFT','ARCHIVED')),
    version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
    creation_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (reward_minor * max_participants <= 100000000)
);
CREATE INDEX company_programs_org ON company_programs(organization_id);
CREATE TABLE organization_events (
    id BIGSERIAL PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    actor_id UUID REFERENCES users(id) ON DELETE SET NULL,
    kind TEXT NOT NULL,
    detail JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX organization_events_org ON organization_events(organization_id,id);

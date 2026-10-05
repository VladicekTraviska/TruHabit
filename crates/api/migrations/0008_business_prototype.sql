-- Employer-funded LOCAL simulation rewards. Existing CZK planning fields are unchanged.
ALTER TABLE company_programs DROP CONSTRAINT company_programs_state_check;
ALTER TABLE company_programs ADD CONSTRAINT company_programs_state_check
 CHECK(state IN ('DRAFT','PUBLISHED','CLOSED','ARCHIVED'));
ALTER TABLE company_programs
 ADD COLUMN reward_units BIGINT CHECK(reward_units BETWEEN 1000000 AND 50000000),
 ADD COLUMN budget_units BIGINT NOT NULL DEFAULT 0 CHECK(budget_units>=0),
 ADD COLUMN reserved_units BIGINT NOT NULL DEFAULT 0 CHECK(reserved_units>=0),
 ADD COLUMN paid_units BIGINT NOT NULL DEFAULT 0 CHECK(paid_units>=0),
 ADD COLUMN returned_units BIGINT NOT NULL DEFAULT 0 CHECK(returned_units>=0),
 ADD COLUMN funder_id UUID REFERENCES users(id) ON DELETE SET NULL,
 ADD COLUMN profile TEXT CHECK(profile IN ('LIVE','REPLAY')),
 ADD COLUMN starts_at TIMESTAMPTZ,
 ADD COLUMN ends_at TIMESTAMPTZ,
 ADD COLUMN upload_deadline TIMESTAMPTZ,
 ADD COLUMN review_deadline TIMESTAMPTZ,
 ADD COLUMN published_at TIMESTAMPTZ,
 ADD COLUMN closed_at TIMESTAMPTZ,
 ADD CONSTRAINT company_budget_conserved CHECK(reserved_units+paid_units+returned_units<=budget_units),
 ADD CONSTRAINT company_published_terms CHECK(published_at IS NULL OR
   (reward_units IS NOT NULL AND profile IS NOT NULL AND starts_at IS NOT NULL
    AND ends_at>starts_at AND upload_deadline>=ends_at AND review_deadline>upload_deadline
    AND budget_units=reward_units*max_participants));

CREATE TABLE organization_invitations(
 id UUID PRIMARY KEY,
 organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
 email TEXT NOT NULL,
 role TEXT NOT NULL CHECK(role IN ('ADMIN','MEMBER')),
 token_hash TEXT NOT NULL UNIQUE,
 creation_hash TEXT NOT NULL,
 status TEXT NOT NULL DEFAULT 'OPEN' CHECK(status IN ('OPEN','ACCEPTED','REVOKED')),
 accepted_by UUID REFERENCES users(id) ON DELETE SET NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '7 days'
);
CREATE INDEX organization_invitations_org ON organization_invitations(organization_id,created_at);
CREATE TABLE company_enrollments(
 id UUID PRIMARY KEY,
 program_id UUID NOT NULL REFERENCES company_programs(id) ON DELETE CASCADE,
 user_id UUID REFERENCES users(id) ON DELETE SET NULL,
 state TEXT NOT NULL DEFAULT 'ENROLLED' CHECK(state IN ('ENROLLED','REWARDED','CLOSED')),
 assessment TEXT NOT NULL DEFAULT 'UNKNOWN' CHECK(assessment IN ('UNKNOWN','MET','NOT_MET','REVIEW_REQUIRED')),
 reward_units BIGINT NOT NULL CHECK(reward_units BETWEEN 1000000 AND 50000000),
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 paid_at TIMESTAMPTZ,
 UNIQUE(program_id,user_id)
);
CREATE INDEX company_enrollments_user ON company_enrollments(user_id);
CREATE TABLE company_uploads(
 id UUID PRIMARY KEY,
 enrollment_id UUID NOT NULL REFERENCES company_enrollments(id) ON DELETE CASCADE,
 user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 file_hash TEXT NOT NULL,
 fingerprint TEXT NOT NULL,
 session_index INTEGER,
 content BYTEA CHECK(octet_length(content)<=16777216),
 content_deleted_at TIMESTAMPTZ,
 activity JSONB NOT NULL,
 goal_result TEXT NOT NULL CHECK(goal_result IN ('MET','NOT_MET')),
 decision TEXT NOT NULL CHECK(decision IN ('ACCEPTED','REJECTED','REVIEW_REQUIRED')),
 reason TEXT NOT NULL,
 received_at TIMESTAMPTZ NOT NULL,
 UNIQUE(enrollment_id,file_hash,session_index)
);
CREATE INDEX company_uploads_user ON company_uploads(user_id,file_hash);
CREATE INDEX company_uploads_enrollment ON company_uploads(enrollment_id,received_at);
CREATE TABLE company_enrollment_events(
 id BIGSERIAL PRIMARY KEY,
 enrollment_id UUID NOT NULL REFERENCES company_enrollments(id) ON DELETE CASCADE,
 actor_id UUID REFERENCES users(id) ON DELETE SET NULL,
 kind TEXT NOT NULL,
 detail JSONB NOT NULL DEFAULT '{}',
 created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE prototype_local_movements DROP CONSTRAINT prototype_local_movements_action_check;
ALTER TABLE prototype_local_movements DROP CONSTRAINT prototype_local_movements_check;
ALTER TABLE prototype_local_movements
 ADD COLUMN business_program_id UUID REFERENCES company_programs(id) ON DELETE RESTRICT,
 ADD COLUMN business_enrollment_id UUID REFERENCES company_enrollments(id) ON DELETE RESTRICT,
 ADD COLUMN business_delta BIGINT NOT NULL DEFAULT 0,
 ADD CONSTRAINT local_movement_conserved CHECK(wallet_delta+locked_delta+recipient_delta+issued_delta+business_delta=0),
 ADD CONSTRAINT prototype_local_movements_action_check CHECK(action IN
 ('GRANT','DEPOSIT','SUCCESS','FAILURE','CANCEL','TIMEOUT','BUSINESS_FUND','BUSINESS_PAY','BUSINESS_REWARD','BUSINESS_RELEASE')),
 ADD CONSTRAINT local_business_reference CHECK((action LIKE 'BUSINESS_%')=(business_program_id IS NOT NULL));
CREATE UNIQUE INDEX business_movement_once_program ON prototype_local_movements(business_program_id,action)
 WHERE business_program_id IS NOT NULL AND business_enrollment_id IS NULL;
CREATE UNIQUE INDEX business_movement_once_enrollment ON prototype_local_movements(business_enrollment_id,action)
 WHERE business_enrollment_id IS NOT NULL;

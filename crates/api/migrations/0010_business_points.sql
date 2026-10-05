-- Company benefit points are an independent, explicitly simulated ledger.
ALTER TABLE company_programs
 ADD COLUMN template TEXT NOT NULL DEFAULT 'LEGACY'
   CHECK(template IN ('LEGACY','ACTIVITY_POINTS','EMPLOYER_MATCH','EVENT','MONTHLY_BUDGET')),
 ADD COLUMN point_reward BIGINT NOT NULL DEFAULT 0 CHECK(point_reward BETWEEN 0 AND 100000),
 ADD COLUMN point_stake BIGINT NOT NULL DEFAULT 0 CHECK(point_stake BETWEEN 0 AND 100000),
 ADD COLUMN monthly_decline BOOLEAN NOT NULL DEFAULT false,
 ADD COLUMN previous_cycle_id UUID REFERENCES company_programs(id) ON DELETE SET NULL,
 ADD COLUMN cycle_starts_at TIMESTAMPTZ,
 ADD COLUMN cycle_ends_at TIMESTAMPTZ,
 ADD COLUMN cycle_creation_hash TEXT,
 ADD CONSTRAINT company_point_terms CHECK(
   (template='LEGACY' AND point_reward=0 AND point_stake=0 AND NOT monthly_decline)
   OR (template<>'LEGACY' AND point_reward>=1
       AND (template='EMPLOYER_MATCH' OR point_stake=0)
       AND (template<>'EMPLOYER_MATCH' OR point_stake>=1)
       AND monthly_decline=(template='MONTHLY_BUDGET')));
ALTER TABLE company_programs DROP CONSTRAINT company_programs_reward_units_check;
ALTER TABLE company_programs ADD CONSTRAINT company_programs_reward_units_check
 CHECK(reward_units IS NULL OR
   (template='LEGACY' AND reward_units BETWEEN 1000000 AND 50000000)
   OR (template<>'LEGACY' AND reward_units=point_reward*1000000));
CREATE UNIQUE INDEX company_programs_monthly_child ON company_programs(previous_cycle_id)
 WHERE previous_cycle_id IS NOT NULL;

ALTER TABLE company_enrollments
 ADD COLUMN staked_points BIGINT NOT NULL DEFAULT 0 CHECK(staked_points BETWEEN 0 AND 100000),
 ADD COLUMN awarded_points BIGINT NOT NULL DEFAULT 0 CHECK(awarded_points BETWEEN 0 AND 100000),
 ADD COLUMN consented_at TIMESTAMPTZ,
 ADD COLUMN terms_version INTEGER CHECK(terms_version>0);
ALTER TABLE company_enrollments DROP CONSTRAINT company_enrollments_reward_units_check;
ALTER TABLE company_enrollments ADD CONSTRAINT company_enrollments_reward_units_check
 CHECK(reward_units BETWEEN 1000000 AND 100000000000);

CREATE TABLE business_point_movements(
 id UUID PRIMARY KEY,
 organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
 program_id UUID REFERENCES company_programs(id) ON DELETE CASCADE,
 enrollment_id UUID REFERENCES company_enrollments(id) ON DELETE CASCADE,
 user_id UUID REFERENCES users(id) ON DELETE SET NULL,
 actor_id UUID REFERENCES users(id) ON DELETE SET NULL,
 kind TEXT NOT NULL CHECK(kind IN('TOP_UP','FUND','REWARD','RELEASE','STAKE_LOCK','STAKE_REFUND','STAKE_FORFEIT')),
 pool_delta BIGINT NOT NULL DEFAULT 0,
 reserved_delta BIGINT NOT NULL DEFAULT 0,
 employee_delta BIGINT NOT NULL DEFAULT 0,
 stake_delta BIGINT NOT NULL DEFAULT 0,
 issued_delta BIGINT NOT NULL DEFAULT 0,
 request_hash TEXT,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 CONSTRAINT business_points_conserved CHECK(pool_delta+reserved_delta+employee_delta+stake_delta+issued_delta=0),
 CONSTRAINT business_points_reference CHECK(
   (kind='TOP_UP' AND program_id IS NULL AND enrollment_id IS NULL AND request_hash IS NOT NULL)
   OR (kind IN('FUND','RELEASE') AND program_id IS NOT NULL AND enrollment_id IS NULL)
   OR (kind IN('REWARD','STAKE_LOCK','STAKE_REFUND','STAKE_FORFEIT') AND program_id IS NOT NULL AND enrollment_id IS NOT NULL)),
 CONSTRAINT business_points_kind CHECK(
   (kind='TOP_UP' AND pool_delta>0 AND issued_delta=-pool_delta AND reserved_delta=0 AND employee_delta=0 AND stake_delta=0)
   OR (kind='FUND' AND pool_delta<0 AND reserved_delta=-pool_delta AND employee_delta=0 AND stake_delta=0 AND issued_delta=0)
   OR (kind='RELEASE' AND pool_delta>=0 AND reserved_delta=-pool_delta AND employee_delta=0 AND stake_delta=0 AND issued_delta=0)
   OR (kind='REWARD' AND pool_delta>=0 AND reserved_delta<=0 AND employee_delta>=0 AND stake_delta=0 AND issued_delta=0)
   OR (kind='STAKE_LOCK' AND employee_delta<0 AND stake_delta=-employee_delta AND pool_delta=0 AND reserved_delta=0 AND issued_delta=0)
   OR (kind='STAKE_REFUND' AND employee_delta>0 AND stake_delta=-employee_delta AND pool_delta=0 AND reserved_delta=0 AND issued_delta=0)
   OR (kind='STAKE_FORFEIT' AND pool_delta>0 AND stake_delta=-pool_delta AND employee_delta=0 AND reserved_delta=0 AND issued_delta=0))
);
CREATE INDEX business_points_org ON business_point_movements(organization_id,created_at,id);
CREATE INDEX business_points_user ON business_point_movements(user_id,organization_id);
CREATE UNIQUE INDEX business_points_program_once ON business_point_movements(program_id,kind)
 WHERE program_id IS NOT NULL AND enrollment_id IS NULL;
CREATE UNIQUE INDEX business_points_enrollment_once ON business_point_movements(enrollment_id,kind)
 WHERE enrollment_id IS NOT NULL;

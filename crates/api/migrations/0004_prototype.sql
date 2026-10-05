-- Separate test-token lifecycle; no conversion of existing fiat planning amounts.
CREATE TABLE prototype_operators (
 user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE
);
CREATE TABLE prototype_challenges (
 id UUID PRIMARY KEY,
 user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 title TEXT NOT NULL CHECK(length(title) BETWEEN 1 AND 100),
 target_m INTEGER NOT NULL CHECK(target_m BETWEEN 1000 AND 5000),
 amount_units BIGINT NOT NULL CHECK(amount_units BETWEEN 1000000 AND 50000000),
 profile TEXT NOT NULL CHECK(profile IN ('LIVE','REPLAY')),
 network TEXT NOT NULL CHECK(network IN ('DEVNET','LOCAL')),
 starts_at TIMESTAMPTZ NOT NULL,
 ends_at TIMESTAMPTZ NOT NULL CHECK(ends_at > starts_at),
 upload_deadline TIMESTAMPTZ NOT NULL CHECK(upload_deadline >= ends_at),
 refund_after TIMESTAMPTZ NOT NULL CHECK(refund_after > upload_deadline),
 state TEXT NOT NULL DEFAULT 'DRAFT' CHECK(state IN ('DRAFT','ACTIVE','REFUNDED','FORFEITED','CANCELLED','EXPIRED')),
 assessment TEXT NOT NULL DEFAULT 'UNKNOWN' CHECK(assessment IN ('UNKNOWN','MET','NOT_MET','REVIEW_REQUIRED')),
 policy TEXT NOT NULL DEFAULT 'prototype-distance-v1',
 chain JSONB,
 creation_hash TEXT NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 closed_at TIMESTAMPTZ,
 version INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX prototype_challenges_owner ON prototype_challenges(user_id,created_at);
CREATE TABLE prototype_uploads (
 id UUID PRIMARY KEY,
 challenge_id UUID NOT NULL REFERENCES prototype_challenges(id) ON DELETE CASCADE,
 user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 file_hash TEXT NOT NULL,
 fingerprint TEXT NOT NULL,
 content BYTEA CHECK(octet_length(content)<=16777216),
 activity JSONB NOT NULL,
 goal_result TEXT NOT NULL CHECK(goal_result IN ('MET','NOT_MET','UNKNOWN')),
 decision TEXT NOT NULL CHECK(decision IN ('ACCEPTED','REJECTED','REVIEW_REQUIRED')),
 reason TEXT NOT NULL,
 received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 UNIQUE(user_id,file_hash),
 UNIQUE(user_id,fingerprint)
);
CREATE INDEX prototype_uploads_challenge ON prototype_uploads(challenge_id);
CREATE TABLE prototype_events (
 id BIGSERIAL PRIMARY KEY,
 challenge_id UUID NOT NULL REFERENCES prototype_challenges(id) ON DELETE CASCADE,
 actor_id UUID REFERENCES users(id) ON DELETE SET NULL,
 kind TEXT NOT NULL,
 detail JSONB NOT NULL DEFAULT '{}',
 created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE prototype_commands (
 id UUID PRIMARY KEY,
 challenge_id UUID NOT NULL REFERENCES prototype_challenges(id) ON DELETE CASCADE,
 action TEXT NOT NULL CHECK(action IN ('DEPOSIT','SUCCESS','FAILURE','CANCEL','TIMEOUT')),
 payload JSONB NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('PREPARED','SIGNED','CONFIRMED','EXPIRED','FAILED')),
 signed_transaction TEXT,
 signature TEXT UNIQUE,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX prototype_one_pending ON prototype_commands(challenge_id)
 WHERE status IN ('PREPARED','SIGNED');
CREATE TABLE prototype_local_movements (
 id UUID PRIMARY KEY,
 user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 challenge_id UUID REFERENCES prototype_challenges(id) ON DELETE CASCADE,
 action TEXT NOT NULL CHECK(action IN ('GRANT','DEPOSIT','SUCCESS','FAILURE','CANCEL','TIMEOUT')),
 wallet_delta BIGINT NOT NULL,
 locked_delta BIGINT NOT NULL,
 recipient_delta BIGINT NOT NULL,
 issued_delta BIGINT NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 CHECK(wallet_delta+locked_delta+recipient_delta+issued_delta=0),
 UNIQUE(challenge_id,action)
);

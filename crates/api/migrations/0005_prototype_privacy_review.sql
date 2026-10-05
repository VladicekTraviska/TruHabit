-- Retain the deduplication key and decision when the owner removes a closed
-- challenge's private source file. Removal never changes a settlement.
ALTER TABLE prototype_uploads ALTER COLUMN content DROP NOT NULL;
ALTER TABLE prototype_uploads ADD COLUMN content_deleted_at timestamptz;

CREATE TABLE prototype_review_requests (
    id uuid PRIMARY KEY,
    challenge_id uuid NOT NULL UNIQUE REFERENCES prototype_challenges(id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reason text NOT NULL CHECK (char_length(reason) BETWEEN 5 AND 500),
    status text NOT NULL DEFAULT 'OPEN' CHECK (status IN ('OPEN','CLOSED')),
    resolution text CHECK (char_length(resolution) BETWEEN 5 AND 500),
    created_at timestamptz NOT NULL DEFAULT now(),
    resolved_at timestamptz
);

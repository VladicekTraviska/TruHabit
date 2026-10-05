-- A NOT_MET attempt can never be accepted by manual review. Retain its record,
-- source and original stake, while allowing recovery in a separate challenge.
-- Qualifying evidence remains reserved even if a later reviewer rejects it, so
-- neither file copies nor alternate byte encodings can fund a second success.
ALTER TABLE prototype_uploads
    DROP CONSTRAINT prototype_uploads_user_id_file_hash_key,
    DROP CONSTRAINT prototype_uploads_user_id_fingerprint_key;

CREATE UNIQUE INDEX prototype_uploads_qualifying_file
    ON prototype_uploads(user_id, file_hash) WHERE goal_result = 'MET';
CREATE UNIQUE INDEX prototype_uploads_qualifying_fingerprint
    ON prototype_uploads(user_id, fingerprint) WHERE goal_result = 'MET';

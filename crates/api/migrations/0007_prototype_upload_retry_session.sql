-- Preserve the explicit FIT selection for exact retries after intake closes.
-- Existing uploads retain NULL; no activity, term, or financial state changes.
ALTER TABLE prototype_uploads
    ADD COLUMN session_index INTEGER CHECK (session_index >= 0);

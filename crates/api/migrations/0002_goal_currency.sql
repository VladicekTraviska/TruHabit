-- Existing values remain USD cents. Never reinterpret them as CZK or apply an FX rate.
ALTER TABLE goals ADD COLUMN currency TEXT NOT NULL DEFAULT 'USD'
    CHECK (currency IN ('USD','CZK'));
ALTER TABLE goals DROP CONSTRAINT goals_pledge_cents_check;
ALTER TABLE goals ADD CONSTRAINT goals_amount_currency_check CHECK (
    (currency = 'USD' AND pledge_cents BETWEEN 100 AND 5000)
    OR (currency = 'CZK' AND pledge_cents BETWEEN 5000 AND 100000)
);

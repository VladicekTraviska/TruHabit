//! Validation for personal plans that do not lock a stake.
use crate::RuleError;
pub const POLICY_VERSION: &str = "distance-v1";
pub const DAY: i64 = 86_400;

pub fn validate(
    target_m: i32,
    pledge_cents: i32,
    starts_at: i64,
    now: i64,
) -> Result<(), RuleError> {
    validate_currency(target_m, pledge_cents, "USD", starts_at, now)
}

pub fn validate_currency(
    target_m: i32,
    amount_minor: i32,
    currency: &str,
    starts_at: i64,
    now: i64,
) -> Result<(), RuleError> {
    let amount_valid = match currency {
        "USD" => (100..=5000).contains(&amount_minor),
        "CZK" => (5000..=100000).contains(&amount_minor),
        _ => false,
    };
    if !(1000..=5000).contains(&target_m)
        || !amount_valid
        || starts_at < now.saturating_add(300)
        || starts_at > now.saturating_add(30 * DAY)
    {
        return Err(RuleError::InvalidInput);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn currency_limits_are_not_interchangeable() {
        let now = 1_800_000_000;
        for amount in [5000, 10000, 100000] {
            assert!(validate_currency(3000, amount, "CZK", now + 600, now).is_ok());
        }
        for (amount, currency) in [
            (4999, "CZK"),
            (100001, "CZK"),
            (10000, "USD"),
            (10000, "EUR"),
        ] {
            assert!(validate_currency(3000, amount, currency, now + 600, now).is_err());
        }
    }
    #[test]
    fn accepted_boundaries_and_invalid_values() {
        let now = 1_800_000_000;
        assert!(validate(1000, 100, now + 300, now).is_ok());
        assert!(validate(5000, 5000, now + 30 * DAY, now).is_ok());
        for (distance, amount, start) in [
            (999, 100, now + 300),
            (5001, 100, now + 300),
            (1000, 99, now + 300),
            (1000, 5001, now + 300),
            (1000, 100, now + 299),
            (1000, 100, now + 30 * DAY + 1),
        ] {
            assert!(validate(distance, amount, start, now).is_err());
        }
    }
}

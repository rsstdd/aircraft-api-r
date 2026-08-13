use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Confidence(f32);

impl Confidence {
    pub fn new(value: f32) -> Result<Self, IngestionInvariantError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(IngestionInvariantError::InvalidConfidence)
        }
    }

    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductionYears {
    pub start: Option<i16>,
    pub end: Option<i16>,
}

impl ProductionYears {
    pub fn new(start: Option<i16>, end: Option<i16>) -> Result<Self, IngestionInvariantError> {
        if let (Some(start), Some(end)) = (start, end)
            && end < start
        {
            return Err(IngestionInvariantError::ProductionYearsReversed { start, end });
        }
        for year in [start, end].into_iter().flatten() {
            if !(1800..=2200).contains(&year) {
                return Err(IngestionInvariantError::YearOutOfRange(year));
            }
        }
        Ok(Self { start, end })
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum IngestionInvariantError {
    #[error("confidence must be finite and between zero and one")]
    InvalidConfidence,
    #[error("production end year {end} is earlier than start year {start}")]
    ProductionYearsReversed { start: i16, end: i16 },
    #[error("year {0} is outside the supported aircraft-data range")]
    YearOutOfRange(i16),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_reversed_production_years() {
        assert_eq!(
            ProductionYears::new(Some(2000), Some(1999)),
            Err(IngestionInvariantError::ProductionYearsReversed { start: 2000, end: 1999 })
        );
    }

    #[test]
    fn confidence_is_bounded() {
        assert!(Confidence::new(0.2).is_ok());
        assert_eq!(Confidence::new(1.1), Err(IngestionInvariantError::InvalidConfidence));
    }
}

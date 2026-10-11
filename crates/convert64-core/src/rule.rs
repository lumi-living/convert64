//! The validator rules of `docs/reference/vic-ii-constraints.md` §6, numbered as there.

use serde::Serialize;
use std::fmt;

/// One of the validator rules 1–5 in `docs/reference/vic-ii-constraints.md` §6.
///
/// Rule 6 (file sizes) belongs to the format readers and writers, not to the image model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(into = "u8")]
pub enum Rule {
    /// 1. Colours per cell never exceeded, counting global colours: 2 (hi-res modes) or
    ///    4 (multicolor modes).
    ColoursPerCell,
    /// 2. All colour values are palette indices 0–15; multicolor-charset per-char colour
    ///    is 0–7.
    ColourRange,
    /// 3. Charset outputs respect the unique-character budget (256, or 64 for ECM) and
    ///    reference only existing chars.
    CharsetBudget,
    /// 4. Sprite data is exactly 64 bytes per sprite; MC1/MC2 are single values for the
    ///    whole set; overlay pairs share coordinates.
    Sprites,
    /// 5. Exported addresses satisfy the alignment table and avoid the char-ROM shadow
    ///    ranges.
    Placement,
}

impl Rule {
    /// The rule's number in the reference document.
    pub const fn number(self) -> u8 {
        match self {
            Self::ColoursPerCell => 1,
            Self::ColourRange => 2,
            Self::CharsetBudget => 3,
            Self::Sprites => 4,
            Self::Placement => 5,
        }
    }

    /// The rule's text from the reference document.
    pub const fn text(self) -> &'static str {
        match self {
            Self::ColoursPerCell => {
                "Colours per cell never exceeded, counting global colours: 2 (hi-res modes) or 4 (multicolor modes)."
            }
            Self::ColourRange => {
                "All colour values are palette indices 0-15; multicolor-charset per-char colour is 0-7."
            }
            Self::CharsetBudget => {
                "Charset outputs respect the unique-character budget (256, or 64 for ECM) and reference only existing chars."
            }
            Self::Sprites => {
                "Sprite data is exactly 64 bytes per sprite; MC1/MC2 are single values for the whole set; overlay pairs share coordinates."
            }
            Self::Placement => {
                "Exported addresses satisfy the alignment table and avoid the char-ROM shadow ranges."
            }
        }
    }
}

impl From<Rule> for u8 {
    fn from(rule: Rule) -> Self {
        rule.number()
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rule {}", self.number())
    }
}

#[cfg(test)]
mod tests {
    use super::Rule;

    #[test]
    fn numbers_match_the_reference() {
        let all = [
            Rule::ColoursPerCell,
            Rule::ColourRange,
            Rule::CharsetBudget,
            Rule::Sprites,
            Rule::Placement,
        ];
        let numbers: Vec<u8> = all.iter().map(|r| r.number()).collect();
        assert_eq!(numbers, [1, 2, 3, 4, 5]);
        assert_eq!(Rule::Placement.to_string(), "rule 5");
        assert!(Rule::CharsetBudget.text().contains("64 for ECM"));
    }
}

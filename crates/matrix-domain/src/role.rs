use std::cmp::Ordering;

/// Ruolo di un membro, derivato dal power level Matrix (0 / 50 / 100).
/// Ordine derivato: Admin < Moderator < Member, così `sort` mette gli admin per primi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum MemberRole {
    Admin,
    Moderator,
    #[default]
    Member,
}

impl MemberRole {
    /// PL qualunque → ruolo: >=100 Admin, >=50 Moderatore, altrimenti (anche negativi) Membro.
    pub fn from_power_level(pl: i64) -> Self {
        match pl {
            100.. => Self::Admin,
            50..=99 => Self::Moderator,
            _ => Self::Member,
        }
    }

    pub fn power_level(self) -> i64 {
        match self {
            Self::Admin => 100,
            Self::Moderator => 50,
            Self::Member => 0,
        }
    }
}

/// Ordina per ruolo (Admin, Moderatore, Membro), poi per nome (case-insensitive, poi esatto per stabilità).
pub fn cmp_by_role_then_name(a: (MemberRole, &str), b: (MemberRole, &str)) -> Ordering {
    a.0.cmp(&b.0)
        .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
        .then_with(|| a.1.cmp(b.1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use MemberRole::*;

    #[test]
    fn thresholds() {
        for (pl, r) in [
            (i64::MIN, Member), (-1, Member), (0, Member), (1, Member), (49, Member),
            (50, Moderator), (99, Moderator), (100, Admin), (101, Admin), (i64::MAX, Admin),
        ] {
            assert_eq!(MemberRole::from_power_level(pl), r, "pl {pl}");
        }
    }

    #[test]
    fn round_trip() {
        for r in [Admin, Moderator, Member] {
            assert_eq!(MemberRole::from_power_level(r.power_level()), r);
        }
        assert_eq!([Admin.power_level(), Moderator.power_level(), Member.power_level()], [100, 50, 0]);
    }

    #[test]
    fn ordering() {
        let mut v = vec![(Member, "zoe"), (Admin, "bob"), (Member, "Anna"), (Moderator, "carl"), (Admin, "alice")];
        v.sort_by(|a, b| cmp_by_role_then_name(*a, *b));
        assert_eq!(
            v,
            vec![(Admin, "alice"), (Admin, "bob"), (Moderator, "carl"), (Member, "Anna"), (Member, "zoe")]
        );
    }
}

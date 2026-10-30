//! The three-way decision for one file in one sync folder.
//!
//! Each side is known by its content hash. The **base** is the hash this
//! machine and the folder last agreed on; it is what tells "I changed it"
//! from "they changed it". The table, in the order it is checked:
//!
//! | Local | Folder | Decision |
//! | --- | --- | --- |
//! | any | not there | [`Decision::Push`], or [`Decision::Nothing`] without a local file |
//! | same as the folder | | [`Decision::InSync`] |
//! | no file | there | [`Decision::Pull`]: a sync never deletes |
//! | blank (no rules), never synced or unchanged since | changed | [`Decision::Pull`] |
//! | no base | different | [`Decision::Conflict`] ([`ConflictKind::Join`]) |
//! | unchanged since the base | changed | [`Decision::Pull`] |
//! | changed | unchanged since the base | [`Decision::Push`] |
//! | changed | changed differently | [`Decision::Conflict`] ([`ConflictKind::BothChanged`]) |
//!
//! Nothing here reads a file; [`crate::pass`] gathers the facts.

/// The local file as the decision sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Local<'a> {
    /// No local file.
    Missing,
    /// A file that says nothing yet (for `rules.json`, no rules and no
    /// default), which a machine joining a folder gives up without asking.
    Blank(&'a str),
    Present(&'a str),
}

/// Why two sides cannot be put together without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// This machine has never synced with the folder, and both have rules.
    Join,
    /// Both changed since they last agreed.
    BothChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Neither side has the file.
    Nothing,
    /// Both sides hold the same bytes; that hash becomes the base.
    InSync,
    /// Copy the local file to the folder.
    Push,
    /// Copy the folder's file over the local one.
    Pull,
    Conflict(ConflictKind),
}

/// Decides what to do for one file. `remote` is the folder's hash, `None`
/// when the folder has no such file; `base` the last agreed hash.
pub fn decide(local: Local<'_>, remote: Option<&str>, base: Option<&str>) -> Decision {
    let Some(remote) = remote else {
        return match local {
            Local::Missing => Decision::Nothing,
            _ => Decision::Push,
        };
    };
    let local_hash = match local {
        Local::Missing => return Decision::Pull,
        Local::Blank(h) | Local::Present(h) => h,
    };
    if local_hash == remote {
        return Decision::InSync;
    }
    if matches!(local, Local::Blank(_)) && (base.is_none() || base == Some(local_hash)) {
        return Decision::Pull;
    }
    let Some(base) = base else {
        return Decision::Conflict(ConflictKind::Join);
    };
    if local_hash == base {
        Decision::Pull
    } else if remote == base {
        Decision::Push
    } else {
        Decision::Conflict(ConflictKind::BothChanged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ConflictKind::*;
    use Decision::*;
    use Local::*;

    #[test]
    fn nothing_in_the_folder_is_a_push() {
        assert_eq!(decide(Present("a"), None, None), Push);
        assert_eq!(decide(Present("a"), None, Some("a")), Push, "folder deleted");
        assert_eq!(decide(Blank("e"), None, None), Push);
        assert_eq!(decide(Missing, None, None), Nothing);
        assert_eq!(decide(Missing, None, Some("a")), Nothing);
    }

    #[test]
    fn the_same_content_is_in_sync_whatever_the_base() {
        assert_eq!(decide(Present("a"), Some("a"), None), InSync);
        assert_eq!(decide(Present("a"), Some("a"), Some("z")), InSync);
        assert_eq!(decide(Blank("e"), Some("e"), None), InSync);
    }

    #[test]
    fn one_side_changed_copies_it() {
        assert_eq!(decide(Present("a"), Some("b"), Some("a")), Pull);
        assert_eq!(decide(Present("b"), Some("a"), Some("a")), Push);
    }

    #[test]
    fn both_changed_is_a_conflict() {
        assert_eq!(
            decide(Present("b"), Some("c"), Some("a")),
            Conflict(BothChanged)
        );
    }

    #[test]
    fn a_new_machine_takes_the_folders_rules_only_when_it_has_none() {
        assert_eq!(decide(Missing, Some("r"), None), Pull);
        assert_eq!(decide(Blank("e"), Some("r"), None), Pull);
        assert_eq!(decide(Present("l"), Some("r"), None), Conflict(Join));
    }

    #[test]
    fn a_blank_local_that_changed_since_the_base_is_still_an_edit() {
        // The user deleted every rule here while another machine edited:
        // that is a conflict, not a silent pull.
        assert_eq!(
            decide(Blank("e"), Some("r2"), Some("r1")),
            Conflict(BothChanged)
        );
        // Blank and unchanged since the base: theirs.
        assert_eq!(decide(Blank("e"), Some("r2"), Some("e")), Pull);
        // A deleted local file is never pushed as a deletion: the folder's
        // copy comes back.
        assert_eq!(decide(Missing, Some("r2"), Some("r1")), Pull);
        assert_eq!(decide(Missing, Some("r1"), Some("r1")), Pull);
    }
}

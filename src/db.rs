//! SQLite persistence using the exact schema of the Android app
//! (`NFCIDTable`: NFCID PK, CODENAME, UNAME, EMAIL, COFF).

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

use crate::protocol::{codename_to_nfcid, nfcid_to_codename};

pub const TABLE: &str = "NFCIDTable";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct User {
    pub nfcid: String,
    pub codename: String,
    pub uname: String,
    pub email: String,
    pub coff: i64,
}

#[derive(Debug, Default, Clone)]
pub struct MergeSummary {
    pub inserted: usize,
    pub updated: usize,
    pub coffees_added: u64,
}

impl MergeSummary {
    pub fn one_line(&self) -> String {
        format!(
            "{} new, {} updated, +{} coffees",
            self.inserted, self.updated, self.coffees_added
        )
    }
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening database {}", path.display()))?;
        conn.execute_batch(
            &format!(
                "CREATE TABLE IF NOT EXISTS {TABLE} (
                    NFCID TEXT NOT NULL PRIMARY KEY,
                    CODENAME TEXT NOT NULL,
                    UNAME TEXT,
                    EMAIL TEXT,
                    COFF INT
                );"
            ),
        )?;
        Ok(Self { conn })
    }

    pub fn list_users(&self) -> Result<Vec<User>> {
        let mut stmt = self.conn.prepare(
            "SELECT NFCID, CODENAME, UNAME, EMAIL, COFF \
             FROM NFCIDTable \
             ORDER BY UNAME COLLATE NOCASE ASC, CODENAME ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(User {
                nfcid: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                codename: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                uname: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                email: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                coff: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
            })
        })?;
        let mut out = Vec::new();
        for u in rows {
            out.push(u?);
        }
        Ok(out)
    }

    /// Insert or replace a user, keyed by NFCID (Android primary key).
    pub fn upsert(&self, u: &User) -> Result<()> {
        self.conn.execute(
            &format!(
                "INSERT INTO {TABLE} (NFCID, CODENAME, UNAME, EMAIL, COFF) \
                 VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT(NFCID) DO UPDATE SET \
                   CODENAME = excluded.CODENAME, UNAME = excluded.UNAME, \
                   EMAIL = excluded.EMAIL, COFF = excluded.COFF"
            ),
            params![u.nfcid, u.codename, u.uname, u.email, u.coff],
        )?;
        Ok(())
    }

    pub fn delete(&self, codename: &str) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM NFCIDTable WHERE CODENAME = ?1",
            params![codename],
        )?;
        Ok(n)
    }

    pub fn clear_counters(&self) -> Result<usize> {
        let n = self.conn.execute("UPDATE NFCIDTable SET COFF = 0", [])?;
        Ok(n)
    }

    /// Merge a completed transfer: `new = existing + received` (Android rule).
    pub fn apply_transfer(&self, records: &[(String, u32)]) -> Result<MergeSummary> {
        let mut summary = MergeSummary::default();
        let tx = self.conn.unchecked_transaction()?;
        for (codename, count) in records {
            let existing = Self::coff_for(&tx, codename)?;
            let new_coff = existing.unwrap_or(0) + *count as i64;
            let nfcid = codename_to_nfcid(codename).unwrap_or_default();
            if existing.is_some() {
                summary.updated += 1;
            } else {
                summary.inserted += 1;
            }
            summary.coffees_added += *count as u64;
            tx.execute(
                &format!(
                    "INSERT INTO {TABLE} (NFCID, CODENAME, UNAME, EMAIL, COFF) \
                     VALUES (?1, ?2, '', '', ?3) \
                     ON CONFLICT(NFCID) DO UPDATE SET \
                       CODENAME = excluded.CODENAME, COFF = excluded.COFF"
                ),
                params![nfcid, codename, new_coff],
            )?;
        }
        tx.commit()?;
        Ok(summary)
    }

    fn coff_for(conn: &Connection, codename: &str) -> Result<Option<i64>> {
        let v = conn
            .query_row(
                "SELECT COFF FROM NFCIDTable WHERE CODENAME = ?1",
                params![codename],
                |r| r.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten();
        Ok(v)
    }

    /// Render the database as Android-format CSV.
    pub fn export_csv(&self) -> Result<String> {
        let users = self.list_users()?;
        let mut out = String::from("NFCID,CODENAME,UNAME,EMAIL,COFF\n");
        for u in users {
            out.push_str(&format!(
                "{},{},{},{},{}\n",
                u.nfcid, u.codename, u.uname, u.email, u.coff
            ));
        }
        Ok(out)
    }

    /// Import Android-format CSV text. Matches the Android import semantics:
    /// `existing + row_coff`. The header line is validated when present.
    pub fn import_csv(&self, text: &str) -> Result<MergeSummary> {
        let mut summary = MergeSummary::default();
        let tx = self.conn.unchecked_transaction()?;

        for (idx, line) in text.lines().enumerate() {
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() {
                continue;
            }
            if idx == 0 && line.trim().eq_ignore_ascii_case("NFCID,CODENAME,UNAME,EMAIL,COFF")
            {
                continue;
            }
            let cells: Vec<&str> = line.split(',').collect();
            if cells.len() < 5 {
                anyhow::bail!("line {}: expected 5 columns, found {}", idx + 1, cells.len());
            }
            let nfcid_in = cells[0].trim();
            let codename = cells[1].trim().to_string();
            let uname = cells[2].trim().to_string();
            let email = cells[3].trim().to_string();
            let row_coff: i64 = cells[4].trim().parse().with_context(|| {
                format!("line {}: invalid coffee count '{}'", idx + 1, cells[4])
            })?;

            if codename.is_empty() {
                anyhow::bail!("line {}: empty CODENAME", idx + 1);
            }

            // NFCID: prefer the CSV value; derive from the code name if absent.
            let nfcid = if nfcid_in.is_empty() {
                codename_to_nfcid(&codename).unwrap_or_default()
            } else {
                nfcid_in.to_string()
            };

            let existing = Self::coff_for(&tx, &codename)?;
            let new_coff = existing.unwrap_or(0) + row_coff;
            if existing.is_some() {
                summary.updated += 1;
            } else {
                summary.inserted += 1;
            }
            summary.coffees_added += row_coff.max(0) as u64;

            tx.execute(
                &format!(
                    "INSERT INTO {TABLE} (NFCID, CODENAME, UNAME, EMAIL, COFF) \
                     VALUES (?1, ?2, ?3, ?4, ?5) \
                     ON CONFLICT(NFCID) DO UPDATE SET \
                       CODENAME = excluded.CODENAME, UNAME = excluded.UNAME, \
                       EMAIL = excluded.EMAIL, COFF = excluded.COFF"
                ),
                params![nfcid, codename, uname, email, new_coff],
            )?;
        }
        tx.commit()?;
        Ok(summary)
    }
}

/// Derive a canonical `User` from whatever key fields are supplied.
/// Fills in NFCID/CODENAME from one another where possible.
pub fn normalize_user(
    nfcid: &str,
    codename: &str,
    uname: &str,
    email: &str,
    coff: i64,
) -> Result<User, String> {
    let (nfcid, codename) = match (nfcid.trim().is_empty(), codename.trim().is_empty()) {
        (false, false) => (nfcid.trim().to_uppercase(), codename.trim().to_uppercase()),
        (false, true) => {
            let n = nfcid.trim().to_uppercase();
            let c = nfcid_to_codename(&n)?;
            (n, c)
        }
        (true, false) => {
            let c = codename.trim().to_uppercase();
            let n = codename_to_nfcid(&c)?;
            (n, c)
        }
        (true, true) => return Err("enter an NFCID (hex) or a CODENAME".to_string()),
    };
    Ok(User {
        nfcid,
        codename,
        uname: uname.to_string(),
        email: email.to_string(),
        coff,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db() -> (Db, std::path::PathBuf) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("nfcoffee_test_{nanos}.db"));
        let db = Db::open(&path).unwrap();
        (db, path)
    }

    #[test]
    fn transfer_adds_to_existing_count() {
        let (db, path) = temp_db();
        let code = "00001FY0.V1E";
        let s1 = db.apply_transfer(&[(code.to_string(), 5)]).unwrap();
        assert_eq!(s1.inserted, 1);
        assert_eq!(s1.coffees_added, 5);
        let s2 = db.apply_transfer(&[(code.to_string(), 3)]).unwrap();
        assert_eq!(s2.updated, 1);
        let users = db.list_users().unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].coff, 8);
        assert_eq!(users[0].nfcid, "BB36AB22");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn csv_roundtrip_and_import_adds() {
        let (db, path) = temp_db();
        let code = "00001FY0.V1E";
        db.apply_transfer(&[(code.to_string(), 2)]).unwrap();
        db.upsert(&User {
            nfcid: "BB36AB22".into(),
            codename: code.into(),
            uname: "Alice".into(),
            email: "alice@example.com".into(),
            coff: 2,
        })
        .unwrap();

        let csv = db.export_csv().unwrap();
        assert!(csv.starts_with("NFCID,CODENAME,UNAME,EMAIL,COFF\n"));
        assert!(csv.contains("BB36AB22,00001FY0.V1E,Alice,alice@example.com,2"));

        // Import adds to the existing value (Android semantics).
        let text = "NFCID,CODENAME,UNAME,EMAIL,COFF\nBB36AB22,00001FY0.V1E,Alice,alice@example.com,4\n";
        let s = db.import_csv(text).unwrap();
        assert_eq!(s.updated, 1);
        assert_eq!(db.list_users().unwrap()[0].coff, 6);

        db.clear_counters().unwrap();
        assert_eq!(db.list_users().unwrap()[0].coff, 0);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn import_derives_missing_nfcid() {
        let (db, path) = temp_db();
        let text = "NFCID,CODENAME,UNAME,EMAIL,COFF\n,00001FY0.V1E,Bob,bob@example.com,1\n";
        db.import_csv(text).unwrap();
        let users = db.list_users().unwrap();
        assert_eq!(users[0].nfcid, "BB36AB22");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn normalize_derives_both_directions() {
        let u = normalize_user("BB36AB22", "", "Al", "", 1).unwrap();
        assert_eq!(u.codename, "00001FY0.V1E");
        let u = normalize_user("", "00001FY0.V1E", "Al", "", 1).unwrap();
        assert_eq!(u.nfcid, "BB36AB22");
        assert!(normalize_user("", "", "Al", "", 1).is_err());
    }
}

# Backup and restore

## Make a backup

Settings -> Backup & restore -> **Back up now** (saves to `<app data>/backups`) or **Back up to folder…** (e.g. an external drive).
Each backup is a consistent snapshot taken while the app runs, verified (integrity + foreign keys + schema) before it is published, and written with a manifest `<file>.spbak.manifest.json` containing its SHA-256. Files are never overwritten; every backup has a unique name.
The dashboard warns when the last good backup is older than the reminder period (Settings -> `backup.reminder_days`, default 7, 0 = off).
**Copy backups off this PC** (USB drive or another machine). A backup on the same disk does not protect against disk failure.

## Restore

1. Settings -> Backup & restore -> **Choose backup file…** (needs the `backup.restore` permission, Owner by default).
2. Read the preview: size, creation time, schema, user count, checksum result, warnings. A failed checksum, damaged database, foreign database or a backup from a newer Singer Pro is rejected and cannot be staged.
3. Tick the acknowledgement and **Stage restore**. Singer Pro first saves a safety copy of your current data to `<app data>/pre-restore/`, then stages a verified copy. Nothing in the live database has changed yet.
4. **Restart now to apply** (or close and reopen the app). At startup, before the database opens, the staged file is re-verified and swapped in. Your previous database is moved to `<app data>/replaced/<timestamp>/` and is never deleted automatically.
5. If anything fails at startup the original database is left (or put back) untouched and a message explains why. You can cancel a staged restore at any time before restarting.

Restoring on another PC: install Singer Pro, complete first-run setup (or skip), then restore the `.spbak` file the same way. The restored users and passwords replace the local ones.

## Manual recovery

The live database is `<app data>/data/singerpro.db` (`<app data>` = `%APPDATA%\com.petra.singerpro` on Windows). A `.spbak` file is a normal SQLite database: close Singer Pro, keep a copy of the current `singerpro.db`, and replace it with the backup renamed to `singerpro.db` (delete the `-wal`/`-shm` files next to it).

## Known limitations

- Restore replaces everything, including users and the audit log, with the backup's contents; there is no partial restore.
- Backup history is stored inside the database, so after a restore it shows the history as of that backup plus the restore event.

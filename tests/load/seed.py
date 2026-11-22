#!/usr/bin/env python3
"""Seed a disposable SQLite database for the API read-route load scenario.

API-40 (#801). This script is deliberately local-only: it refuses to touch a
database whose path looks like a production database, and every row it writes is
synthetic (`example.invalid` email domain, fixed load-test Stellar address).

The three tables are created with the same columns as `core/migrations/002`,
`004` and `006` so the scenario is self-contained; the core migrations are
idempotent (`CREATE TABLE IF NOT EXISTS`) and safe to run afterwards.
"""

from __future__ import annotations

import argparse
import os
import sqlite3
import sys
import uuid
from datetime import datetime, timedelta, timezone

ADMIN_MANAGER_ID = "00000000-0000-4000-8000-000000000001"
ADMIN_STELLAR_ADDRESS = "GLOADTESTADMIN0000000000000000000000000000000000000000000000"
EMAIL_DOMAIN = "example.invalid"

DDL = (
    """
    CREATE TABLE IF NOT EXISTS ledger_fee_samples (
        ledger_sequence INTEGER PRIMARY KEY,
        collected_at TEXT NOT NULL,
        base_reserve INTEGER NOT NULL,
        base_fee INTEGER NOT NULL,
        max_fee INTEGER NOT NULL,
        fee_charged INTEGER NOT NULL,
        transaction_count INTEGER NOT NULL,
        ledger_close_time TEXT NOT NULL
    )
    """,
    """
    CREATE TABLE IF NOT EXISTS vaults (
        id TEXT PRIMARY KEY,
        manager_id TEXT NOT NULL,
        name TEXT NOT NULL,
        status TEXT NOT NULL DEFAULT 'active',
        config_json TEXT NOT NULL DEFAULT '{}',
        version INTEGER NOT NULL DEFAULT 1,
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
    )
    """,
    """
    CREATE TABLE IF NOT EXISTS managers (
        id TEXT PRIMARY KEY,
        stellar_address TEXT NOT NULL UNIQUE,
        name TEXT NOT NULL,
        email TEXT NOT NULL DEFAULT '',
        status TEXT NOT NULL DEFAULT 'pending'
            CHECK (status IN ('pending', 'approved', 'rejected')),
        kyc_document_ref TEXT NOT NULL DEFAULT '',
        notes TEXT NOT NULL DEFAULT '',
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
    )
    """,
)


def utc_iso(moment: datetime) -> str:
    return moment.astimezone(timezone.utc).isoformat(timespec="microseconds").replace("+00:00", "Z")


def safe_db_path(path: str) -> bool:
    lowered = path.lower()
    return "prod" not in lowered and path.startswith(("loadtest", "test"))


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", required=True, help="Path to a disposable SQLite file")
    parser.add_argument("--vaults", type=int, default=2000)
    parser.add_argument("--managers", type=int, default=200)
    parser.add_argument("--fee-samples", type=int, default=1000)
    parser.add_argument("--force", action="store_true", help="Allow a non-loadtest --db path")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if os.environ.get("APP_ENV", "").lower() in {"production", "prod"}:
        print("refusing to seed with APP_ENV=production", file=sys.stderr)
        return 2
    if not args.force and not safe_db_path(args.db):
        print(
            "refusing to seed --db outside a disposable loadtest/test path "
            "(use --force only for a dedicated test database)",
            file=sys.stderr,
        )
        return 2
    if min(args.vaults, args.managers, args.fee_samples) < 1:
        print("--vaults, --managers and --fee-samples must all be >= 1", file=sys.stderr)
        return 2

    now = datetime.now(timezone.utc)
    conn = sqlite3.connect(args.db)
    try:
        with conn:
            for statement in DDL:
                conn.execute(statement)

            manager_rows = []
            for index in range(args.managers):
                if index == 0:
                    manager_id = ADMIN_MANAGER_ID
                    address = ADMIN_STELLAR_ADDRESS
                else:
                    manager_id = str(uuid.uuid5(uuid.NAMESPACE_URL, f"loadtest-manager-{index}"))
                    address = f"GLOADTESTMGR{index:016X}"
                created = utc_iso(now - timedelta(days=args.managers - index))
                manager_rows.append(
                    (
                        manager_id,
                        address,
                        f"loadtest-manager-{index}",
                        f"loadtest-{index}@{EMAIL_DOMAIN}",
                        "approved",
                        "",
                        "",
                        created,
                        created,
                    )
                )
            conn.executemany(
                """INSERT OR REPLACE INTO managers
                   (id, stellar_address, name, email, status, kyc_document_ref,
                    notes, created_at, updated_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)""",
                manager_rows,
            )

            vault_rows = []
            for index in range(args.vaults):
                created = utc_iso(now - timedelta(minutes=args.vaults - index))
                vault_rows.append(
                    (
                        str(uuid.uuid5(uuid.NAMESPACE_URL, f"loadtest-vault-{index}")),
                        manager_rows[index % len(manager_rows)][0],
                        f"loadtest-vault-{index:06d}",
                        "active",
                        '{"strategy":"synthetic","testnet":true,"load_test":true}',
                        1,
                        created,
                        created,
                    )
                )
            conn.executemany(
                """INSERT OR REPLACE INTO vaults
                   (id, manager_id, name, status, config_json, version, created_at, updated_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?)""",
                vault_rows,
            )

            fee_rows = []
            base_ledger = 1_000_000
            for index in range(args.fee_samples):
                stamp = utc_iso(now - timedelta(minutes=args.fee_samples - index))
                fee_rows.append(
                    (
                        base_ledger + index,
                        stamp,
                        100,
                        100 + (index % 5),
                        10_000,
                        100 + (index % 5),
                        10 + (index % 7),
                        stamp,
                    )
                )
            conn.executemany(
                """INSERT OR REPLACE INTO ledger_fee_samples
                   (ledger_sequence, collected_at, base_reserve, base_fee, max_fee,
                    fee_charged, transaction_count, ledger_close_time)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?)""",
                fee_rows,
            )
    finally:
        conn.close()

    print(
        "seeded "
        f"{args.managers} managers, {args.vaults} vaults, {args.fee_samples} fee samples "
        f"into {args.db}"
    )
    print(f"LOAD_TEST_MANAGER_ID={ADMIN_MANAGER_ID}")
    print(f"LOAD_TEST_ADMIN_ADDRESS={ADMIN_STELLAR_ADDRESS}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Manual application analytics acceptance using the real UniFFI query API."""
import argparse, importlib.util, shutil, sqlite3, tempfile, sys
from pathlib import Path

def main():
    p = argparse.ArgumentParser()
    p.add_argument("--bindings", type=Path, required=True)
    p.add_argument("--library", type=Path, required=True)
    p.add_argument("--resources", type=Path, required=True)
    args = p.parse_args()
    with tempfile.TemporaryDirectory(prefix="velune-analytics-") as temp:
        root = Path(temp); bind = root / "bindings"; bind.mkdir()
        shutil.copy2(args.bindings / "velune_bindings.py", bind)
        (bind / "libvelune_bindings.dylib").symlink_to(args.library.resolve())
        spec = importlib.util.spec_from_file_location("velune_bindings", bind / "velune_bindings.py")
        module = importlib.util.module_from_spec(spec); sys.modules[spec.name] = module; spec.loader.exec_module(module)
        home = root / "home"; home.mkdir()
        db = sqlite3.connect(home / "analytics.sqlite")
        db.execute("""CREATE TABLE request_usage (request_id TEXT PRIMARY KEY, provider_id TEXT, provider_name TEXT, model_record_key TEXT, provider_model_id TEXT, protocol TEXT, started_at_ms INTEGER, terminal_at_ms INTEGER, elapsed_ms INTEGER, first_output_ms INTEGER, terminal_elapsed_ms INTEGER, status INTEGER, outcome TEXT, input_tokens INTEGER, output_tokens INTEGER, reasoning_output_tokens INTEGER, cached_input_tokens INTEGER, cache_read_input_tokens INTEGER, cache_creation_input_tokens INTEGER, usage_reported INTEGER, usage_complete INTEGER, uncached_input_tokens INTEGER)""")
        db.execute("PRAGMA user_version=1")
        rows = [("ok","p1","Provider","m1","upstream-1","responses",0,1000,1000,250,1000,200,"completed",10,20,3,2,None,None,1,1),("failed","p1","Provider","m1","upstream-1","responses",0,1200,1200,None,None,502,"failed",10,99,None,None,None,None,1,0),("cancelled","p2","Other","m2","upstream-2","messages",0,1400,1400,None,None,None,"cancelled",None,None,None,None,None,None,0,0)]
        db.executemany("INSERT INTO request_usage VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)", [row+(None,) for row in rows]); db.commit(); db.close()
        app = module.VeluneApplication.open(module.BindingOptions(home_directory=str(home), resources_directory=str(args.resources.resolve())))
        query = module.BindingAnalyticsQuery(from_ms=0, to_ms=2000, bucket_boundaries_ms=[0,1000,2000], provider_id=None, model_record_key=None, request_limit=2)
        report = app.analytics_query(query)
        assert report.overview.request_count == 3 and len(report.requests) == 2
        assert report.overview.eligible_speed_count == 1 and report.overview.output_tokens_per_second == 20.0
        assert report.overview.total_tokens == 139
        app.shutdown()
        reopened = module.VeluneApplication.open(module.BindingOptions(home_directory=str(home), resources_directory=str(args.resources.resolve())))
        report2 = reopened.analytics_query(query)
        assert report2.overview.request_count == 3 and len(report2.requests) == 2
        reopened.shutdown()
        # More than the former population cap: pagination must not limit totals.
        db = sqlite3.connect(home / "analytics.sqlite")
        db.executemany("INSERT INTO request_usage VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)", [(f"bulk-{i}",)+rows[0][1:]+(None,) for i in range(100001)])
        db.commit(); db.close()
        large = module.VeluneApplication.open(module.BindingOptions(home_directory=str(home), resources_directory=str(args.resources.resolve())))
        full = large.analytics_query(query)
        assert full.overview.request_count == 100004 and len(full.requests) == 2
        assert full.overview.eligible_speed_count == 100002
        large.shutdown()
        db = sqlite3.connect(home / "analytics.sqlite")
        db.execute("UPDATE request_usage SET input_tokens=-1 WHERE request_id='ok'"); db.commit(); db.close()
        invalid = module.VeluneApplication.open(module.BindingOptions(home_directory=str(home), resources_directory=str(args.resources.resolve())))
        try:
            invalid.analytics_query(query)
        except module.BindingError:
            pass
        else:
            raise AssertionError("negative stored tokens must not wrap")
        invalid.shutdown()
        broken_home = root / "broken"; broken_home.mkdir()
        (broken_home / "analytics.sqlite").write_bytes(b"not a database")
        available = module.VeluneApplication.open(module.BindingOptions(home_directory=str(broken_home), resources_directory=str(args.resources.resolve())))
        broken = available.analytics_query(query)
        assert broken.storage_warning and broken.overview.request_count == 0
        available.shutdown()
    print("PASS real UniFFI Application analytics query, full aggregation, bounded requests, reopen, 100004-row population, invalid numeric data, unavailable store")
if __name__ == "__main__": main()

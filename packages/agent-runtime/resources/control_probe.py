# Native lock observation, run on the execution machine. Never creates a lock,
# removes a stale file, kills a holder, or resumes a session to test ownership.
import json, os, sys
r = json.load(sys.stdin)
p = r['path']
try:
    if os.name != 'nt':
        import fcntl
        if r['kind'] == 'deepseek': p = os.path.join(p, 'session.lock')
        with open(p, 'rb') as f:
            try:
                fcntl.flock(f.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                fcntl.flock(f.fileno(), fcntl.LOCK_UN)
                result = 'available'
            except BlockingIOError: result = 'heldElsewhere'
    else: result = 'unknown'
except FileNotFoundError: result = 'available'
except Exception: result = 'unknown'
print(json.dumps(result))

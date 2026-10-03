#!/usr/bin/env python3
"""Check that the published request schema equals the running Rust contract."""
import json
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parent.parent
binary = sys.argv[1] if len(sys.argv) > 1 else str(root / 'target/release/avw')
result = subprocess.run([binary, 'schema'], capture_output=True, text=True, check=True)
schema = json.loads(result.stdout)
assert schema['ok'], schema
assert schema['result'] == json.loads((root / 'specs/schemas/service-request.schema.json').read_text()), 'Regenerate service-request.schema.json from avw schema'
print('Published request schema matches Rust service')

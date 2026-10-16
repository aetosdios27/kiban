"""Read-only public-API capability probe, never signs up or creates resources."""
import json
import os
import urllib.request

OPENAPI = 'https://docs.hydradb.com/api-reference/v2/openapi.json'
REASON = ('Hydra hosted v2 API ingests/indexes context asynchronously. The published '
          'API has metadata PATCH but no documented in-place memory-text overwrite '
          'or explicit created_at DESC ordering for memory lists. No faithful W1-W5 '
          'suite or documented same-host no-op is available; no timings reported.')


def probe():
    key = os.environ.get('HYDRADB_API_KEY')
    result = {'backend': 'hydradb', 'status': 'skipped', 'workloads': 'unsupported',
              'reason': REASON, 'openapi': OPENAPI,
              'endpoint': os.environ.get('HYDRADB_ENDPOINT', 'https://api.hydradb.com'),
              'key_present': bool(key)}
    if not key:
        result['reason'] = 'HYDRADB_API_KEY unset. ' + REASON
        return result
    request = urllib.request.Request(result['endpoint'].rstrip('/') + '/databases',
                                     headers={'Authorization': 'Bearer ' + key, 'API-Version': '2'})
    # A successful authentication probe is not a benchmark or a transport floor.
    with urllib.request.urlopen(request, timeout=30) as response:
        payload = json.load(response)
        if not isinstance(payload, dict) or 'data' not in payload:
            raise RuntimeError('Unexpected Hydra databases response')
    result['authenticated_probe'] = 'GET /databases succeeded; body deliberately not recorded'
    return result

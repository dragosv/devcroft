import json
from pathlib import Path
import tempfile
import unittest

from cold_prepare import require_no_active_collector


class PreparationGuard(unittest.TestCase):
    def test_active_calibration_also_blocks_materializer(self):
        with tempfile.TemporaryDirectory() as name:
            runs = Path(name)
            run = runs / 'arbitrary-calibration-name'
            run.mkdir()
            (run / 'runner.pid').write_text('123\n')
            (run / 'report.json').write_text(json.dumps({'status': 'running'}))
            with self.assertRaisesRegex(RuntimeError, 'still active'):
                require_no_active_collector(runs)
            (run / 'report.json').write_text(json.dumps({'status': 'failed'}))
            require_no_active_collector(runs)


if __name__ == '__main__':
    unittest.main()

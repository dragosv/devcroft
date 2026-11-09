"""Adversarial recipe inputs must fail before guest preparation."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from adaptable.recipe import encoded, load, validate, verify_inputs

FIXTURE = Path(__file__).resolve().parent / 'fixtures/adaptable-recipe-aarch64.json'


class RecipeValidation(unittest.TestCase):
    def setUp(self):
        self.recipe = load(FIXTURE)

    def test_real_frozen_recipe_has_a_canonical_identity(self):
        self.assertEqual(encoded(self.recipe), FIXTURE.read_bytes())
        self.assertTrue(all(len(item['sha256']) == 64 for item in self.recipe['packages']))

    def test_missing_content_identity_and_wrong_architecture_fail(self):
        for field, value in [('base_sha256', ''), ('source_sha256', 'latest'), ('architecture', 'aarch64-darwin')]:
            with self.subTest(field=field):
                changed = copy.deepcopy(self.recipe)
                changed[field] = value
                with self.assertRaises(ValueError):
                    validate(changed)
        changed = copy.deepcopy(self.recipe)
        del changed['packages'][0]['sha256']
        with self.assertRaises(ValueError):
            validate(changed)

    def test_omitted_install_inputs_are_not_a_reconstructible_recipe(self):
        for group in ['manager_packages', 'packages']:
            changed = copy.deepcopy(self.recipe)
            changed[group] = []
            with self.assertRaises(ValueError):
                validate(changed)

    def test_unknown_authority_fields_and_network_mechanisms_fail(self):
        changed = copy.deepcopy(self.recipe)
        changed['shell_command'] = 'curl arbitrary | sh'
        with self.assertRaises(ValueError):
            validate(changed)
        changed = copy.deepcopy(self.recipe)
        changed['mechanism'] = 'apk-online'
        with self.assertRaises(ValueError):
            validate(changed)

    def test_filename_escape_duplicate_and_foreign_origin_fail(self):
        for filename in ['../../zlib.apk', '/tmp/zlib.apk', 'a.apk;sh', '.hidden.apk']:
            with self.subTest(filename=filename):
                changed = copy.deepcopy(self.recipe)
                changed['packages'][0]['file'] = filename
                with self.assertRaises(ValueError):
                    validate(changed)
        changed = copy.deepcopy(self.recipe)
        changed['packages'].append(copy.deepcopy(changed['packages'][0]))
        with self.assertRaises(ValueError):
            validate(changed)
        changed = copy.deepcopy(self.recipe)
        changed['packages'][0]['url'] = 'https://example.com/package.apk'
        with self.assertRaises(ValueError):
            validate(changed)

    def test_corrupt_apk_and_symlink_fail_with_working_positive_control(self):
        changed = copy.deepcopy(self.recipe)
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for group in ['manager_packages', 'packages']:
                for item in changed[group]:
                    data = ('fixture APK ' + item['file']).encode()
                    (root / item['file']).write_bytes(data)
                    item['sha256'] = hashlib.sha256(data).hexdigest()
            verify_inputs(changed, root)
            target = root / changed['packages'][0]['file']
            original = target.read_bytes()
            target.write_bytes(original + b'corrupted')
            with self.assertRaises(ValueError):
                verify_inputs(changed, root)
            target.unlink()
            (root / 'symlink-target').write_bytes(original)
            target.symlink_to(root / 'symlink-target')
            with self.assertRaises(ValueError):
                verify_inputs(changed, root)

    def test_oversized_recipe_and_boolean_schema_fail(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'recipe.json'
            path.write_text(json.dumps(self.recipe) + ' ' * 65536)
            with self.assertRaises(ValueError):
                load(path)
        changed = copy.deepcopy(self.recipe)
        changed['schema'] = True
        with self.assertRaises(ValueError):
            validate(changed)


if __name__ == '__main__':
    unittest.main()

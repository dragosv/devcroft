"""A zero helper status, stale acknowledgement or reused PID must not pass as orderly."""
import unittest

from stop_receipt import HelperIdentity, StopReceipt


class StopReceiptValidation(unittest.TestCase):
    def receipt(self, generation=1, nonce='a' * 64):
        return StopReceipt(HelperIdentity(42, 12345), generation, nonce)

    def ack(self, receipt):
        return f'SYNCED {receipt.generation} {receipt.nonce}\n'.encode()

    def test_zero_exit_without_intent_or_ack_is_a_crash(self):
        receipt = self.receipt()
        self.assertEqual(receipt.classify(receipt.identity, 0), 'crash')
        receipt.request(receipt.identity)
        self.assertEqual(receipt.classify(receipt.identity, 0), 'crash')

    def test_acknowledgement_without_intent_is_refused(self):
        receipt = self.receipt()
        self.assertFalse(receipt.accept_ack(receipt.identity, self.ack(receipt)))
        self.assertEqual(receipt.classify(receipt.identity, 0), 'crash')

    def test_correlated_sync_ack_and_clean_exit_are_orderly(self):
        receipt = self.receipt()
        self.assertEqual(receipt.request(receipt.identity), b'SHUTDOWN 1 ' + b'a' * 64 + b'\n')
        self.assertTrue(receipt.accept_ack(receipt.identity, self.ack(receipt)))
        self.assertEqual(receipt.classify(receipt.identity, 0), 'orderly')

    def test_nonzero_exit_and_forced_termination_remain_distinct(self):
        receipt = self.receipt()
        receipt.request(receipt.identity)
        receipt.accept_ack(receipt.identity, self.ack(receipt))
        self.assertEqual(receipt.classify(receipt.identity, 1), 'crash')
        self.assertEqual(receipt.classify(receipt.identity, -15, forced=True), 'forced-stop')

    def test_pid_reuse_is_not_the_recorded_helper(self):
        receipt = self.receipt()
        reused = HelperIdentity(42, 67890)
        with self.assertRaisesRegex(ValueError, 'identity changed'):
            receipt.request(reused)
        receipt.request(receipt.identity)
        self.assertFalse(receipt.accept_ack(reused, self.ack(receipt)))
        self.assertEqual(receipt.classify(reused, 0), 'identity-mismatch')

    def test_old_boot_ack_cannot_authorize_a_new_boot(self):
        old, new = self.receipt(), self.receipt(generation=2)
        old.request(old.identity)
        new.request(new.identity)
        self.assertFalse(new.accept_ack(new.identity, self.ack(old)))
        self.assertEqual(new.classify(new.identity, 0), 'crash')

    def test_wrong_nonce_legacy_bye_and_trailing_data_are_refused(self):
        for packet in [b'BYE\n', b'SYNCED 1 ' + b'b' * 64 + b'\n',
                       b'SYNCED 1 ' + b'a' * 64 + b'\nextra', b'x' * 161]:
            with self.subTest(packet=packet):
                receipt = self.receipt()
                receipt.request(receipt.identity)
                self.assertFalse(receipt.accept_ack(receipt.identity, packet))
                self.assertEqual(receipt.classify(receipt.identity, 0), 'crash')

    def test_receipts_cannot_be_reissued_or_acknowledged_twice(self):
        receipt = self.receipt()
        receipt.request(receipt.identity)
        with self.assertRaises(ValueError):
            receipt.request(receipt.identity)
        self.assertTrue(receipt.accept_ack(receipt.identity, self.ack(receipt)))
        self.assertFalse(receipt.accept_ack(receipt.identity, self.ack(receipt)))

    def test_invalid_identity_generation_and_tokens_fail_closed(self):
        for pid, start in [(0, 1), (1, 0), (True, 1)]:
            with self.assertRaises(ValueError):
                HelperIdentity(pid, start)
        for generation in [0, True, 1000002]:
            with self.assertRaises(ValueError):
                self.receipt(generation=generation)
        for nonce in ['', 'a' * 63, 'G' * 64, 'a' * 64 + '\n']:
            with self.assertRaises(ValueError):
                self.receipt(nonce=nonce)

    def test_running_process_cannot_be_classified_as_exited(self):
        with self.assertRaises(ValueError):
            self.receipt().classify(HelperIdentity(42, 12345), None)


if __name__ == '__main__':
    unittest.main()

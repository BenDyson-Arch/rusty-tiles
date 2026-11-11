"""Independent traces exercise the monitor without its transition generator."""
import unittest

from model import Candidate, Case, Cause, Destination, History, Phase, State, check_edge, replace


class MonitorTests(unittest.TestCase):
    def setUp(self):
        self.case = Case("external_trace", "contract", "no_clobber", ())
        self.initial = State(Phase.STAGING, Candidate.WRITABLE, (), (), Destination.ABSENT)

    def test_every_prepermission_cause_prohibits_permission_and_install(self):
        for action, cause in (("encoder_error", Cause.ENCODER),
                              ("cancel_before_permission", Cause.CANCEL),
                              ("observer_error_before_permission", Cause.OBSERVER),
                              ("publication_error", Cause.PUBLISHER)):
            with self.subTest(cause=cause):
                aborted = replace(self.initial, phase=Phase.ABORTING, primary_cause=cause)
                history, _ = check_edge(self.initial, action, aborted, History(), self.case)
                sealed = replace(aborted, phase=Phase.SEALED, candidate=Candidate.SEALED)
                history, _ = check_edge(aborted, "seal", sealed, history, self.case)
                permitted = replace(sealed, phase=Phase.PUBLISHING)
                history, errors = check_edge(sealed, "grant_publication", permitted, history, self.case)
                self.assertIn("abort_before_permission_prevents_commit", errors)
                committed = replace(permitted, phase=Phase.COMMITTED,
                                    candidate=Candidate.NONE, destination=Destination.OURS)
                _, errors = check_edge(permitted, "install_success", committed, history, self.case)
                self.assertIn("abort_before_permission_prevents_commit", errors)

    def test_producer_cannot_resume_after_seal_or_permission(self):
        for phase in (Phase.SEALED, Phase.PUBLISHING):
            for action in ("encoder_error", "worker_0_finishes"):
                with self.subTest(phase=phase, action=action):
                    before = replace(self.initial, phase=phase, candidate=Candidate.SEALED)
                    after = replace(before, phase=Phase.ABORTING, primary_cause=Cause.ENCODER)
                    history = History(saw_permission=phase == Phase.PUBLISHING)
                    _, errors = check_edge(before, action, after, history, self.case)
                    self.assertIn("no_producer_work_after_seal", errors)

    def test_publisher_failure_after_permission_is_not_prepermission_abort(self):
        permitted = replace(self.initial, phase=Phase.PUBLISHING, candidate=Candidate.SEALED)
        failed = replace(permitted, phase=Phase.ABORTING, primary_cause=Cause.PUBLISHER)
        history, errors = check_edge(permitted, "publication_error", failed, History(saw_permission=True), self.case)
        self.assertEqual([], errors)
        self.assertFalse(history.abort_before_permission)
        self.assertEqual(Cause.PUBLISHER, history.selected_primary)


if __name__ == "__main__":
    unittest.main()

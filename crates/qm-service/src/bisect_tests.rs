#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

fn fix(i: usize) -> BisectCandidate {
    let id = format!("fx_{i}");
    BisectCandidate {
        suspect: TrialSuspect {
            kind: SuspectKind::Fix,
            label: format!("the fix \"F{i}\""),
            name: format!("F{i}"),
            changes: TrialChanges {
                pause_fixes: vec![id.clone()],
                ..TrialChanges::default()
            },
            id,
        },
        source: CandidateSource::RecentFix,
    }
}

fn search(n: usize) -> Bisect {
    Bisect::new(
        "b1".into(),
        "p".into(),
        "r".into(),
        (0..n).map(fix).collect(),
    )
}

/// Runs the search with a culprit at `culprit` (None: not among the candidates) until it
/// ends; returns the trials it took.
fn run(b: &mut Bisect, culprit: Option<usize>) -> usize {
    let mut n = 0;
    while b.end.is_none() {
        let split = b.next_split().expect("a split");
        let result = if culprit.is_some_and(|c| split.contains(&c)) {
            TrialResult::Fine
        } else {
            TrialResult::Crashed
        };
        b.record(b.step(), split, None, result);
        n += 1;
        assert!(n < 20, "the search does not end");
    }
    n
}

#[test]
fn a_culprit_among_eight_fixes_is_found_in_three_trials() {
    for culprit in 0..8 {
        let mut b = search(8);
        assert_eq!(b.view().expected_trials, 3);
        let trials = run(&mut b, Some(culprit));
        let v = b.view();
        assert_eq!(v.end, Some(BisectEnd::Found), "culprit {culprit}");
        assert_eq!(v.found, Some(culprit as u32));
        // The last one is only ever left in, so one more trial confirms it.
        let expected = if culprit == 7 { 4 } else { 3 };
        assert_eq!(trials, expected, "culprit {culprit}");
    }
}

#[test]
fn the_first_trial_leaves_out_the_first_half() {
    let b = search(8);
    let next = b.view().next.unwrap();
    assert_eq!(next.step, 1);
    assert_eq!(next.left_out, vec![0, 1, 2, 3]);
    assert_eq!(
        next.changes.pause_fixes,
        vec!["fx_0", "fx_1", "fx_2", "fx_3"]
    );
    assert_eq!(next.label, "the fix \"F0\", the fix \"F1\" and 2 more");
}

#[test]
fn a_cause_outside_the_candidates_is_not_found() {
    let mut b = search(5);
    run(&mut b, None);
    let v = b.view();
    assert_eq!(v.end, Some(BisectEnd::NotFound));
    assert_eq!(v.found, None);
    assert!(v.remaining.is_empty());
}

#[test]
fn two_unknown_results_stop_with_could_not_tell() {
    let mut b = search(4);
    let split = b.next_split().unwrap();
    b.record(1, split.clone(), None, TrialResult::Unknown);
    // The same split runs again.
    assert_eq!(b.next_split(), Some(split.clone()));
    assert_eq!(b.end, None);
    b.record(2, split, None, TrialResult::Unknown);
    let v = b.view();
    assert_eq!(v.end, Some(BisectEnd::CouldNotTell));
    assert_eq!(v.message.as_deref(), Some(MSG_TWO_UNKNOWN));
    assert_eq!(v.next, None);
}

#[test]
fn a_split_that_breaks_a_file_is_skipped() {
    let mut b = search(4);
    b.skip(vec![0, 1]);
    assert_eq!(b.next_split(), Some(vec![2, 3]));
    assert_eq!(b.view().skipped, 1);
    // Leaving out the second half: a crash means the cause is in the first half.
    b.record(1, vec![2, 3], None, TrialResult::Crashed);
    assert_eq!(b.remaining, vec![0, 1]);
    b.skip(vec![0]);
    assert_eq!(b.next_split(), Some(vec![1]));
    b.skip(vec![1]);
    let v = b.view();
    assert_eq!(v.end, Some(BisectEnd::CouldNotTell));
    assert_eq!(v.message.as_deref(), Some(MSG_ALL_BREAK));
}

#[test]
fn one_candidate_is_tested_once() {
    let mut b = search(1);
    assert_eq!(b.view().expected_trials, 1);
    assert_eq!(run(&mut b, Some(0)), 1);
    assert_eq!(b.view().end, Some(BisectEnd::Found));
}

#[test]
fn halvings_count_trials() {
    assert_eq!(halvings(1), 0);
    assert_eq!(halvings(2), 1);
    assert_eq!(halvings(3), 2);
    assert_eq!(halvings(8), 3);
    assert_eq!(halvings(9), 4);
}

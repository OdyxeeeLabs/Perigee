# Referral System (#664)

On-chain referral binding, volume attribution, and reward claims.

Anti-sybil: max referees, min referee age (ledgers), min volume, no self-referral, immutable bind.

## Anti-sybil limits (#868)

What the contract enforces:

- One referrer per referee, bound once and immutable. A second bind — to the same
  or a competing referrer — fails with `AlreadyReferred` and changes nothing.
- No self-referral. A failed self-referral does not use up the one-time binding.
- `max_referees` caps a referrer; the boundary bind succeeds, the next fails and
  does not increment the count.
- Rejected attributions (`VolumeTooLow`, `RefereeTooNew`, unknown referee) and
  rejected claims (`NothingToClaim`, `InsufficientPool`) change no accounting and
  do not consume the reward pool.
- `attribute_action(referee, action_id, volume)` credits a given `action_id` at
  most once (`DuplicateAction`). The id is only recorded on success.

What it cannot do: it does **not** verify identity. A determined actor with many
addresses can still create many referees up to the cap per referrer, and
mutual referrals (A refers B, B refers A) are allowed. `attribute_volume` has no
action identifier, so a caller that invokes it twice credits twice; use
`attribute_action` when at-most-once matters. Neither attribution entry point
authenticates its caller — restrict who may reach them at the integration layer.

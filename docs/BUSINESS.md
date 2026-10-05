# Team programs

B2B uses the same account system as personal challenges, with separate organization memberships and program participation. New programs use **simulated corporate benefit points**, scoped to their company workspace. No real payment, benefit-provider integration, Devnet transfer or participant Phantom connection takes place. Personal B2C credits are separate.

## Roles and invitation

| Role | Responsibilities |
| --- | --- |
| Owner | Workspace ownership, administrator roles, ownership transfer and program funding |
| Administrator | Member invitations, draft management and permitted program operations |
| Member | Voluntary participation and access to their own activity/result |
| Prototype operator | Explicit separate review permission; can inspect private evidence and resolve eligible reviews or reward claims |

Invitations are delivered manually. The server returns the secret token only on first creation, stores its hash and accepts it only from an account matching the invited email. Tokens expire after seven days and can be revoked. No invitation email is sent automatically. Reviewer provisioning is in [Development](DEVELOPMENT.md).

## Motivation templates

| Template | Successful result | Unmet goal |
| --- | --- | --- |
| Activity Points | The company-funded fixed award is automatically credited | No award; no employee penalty |
| Event | A voluntary shared running event gives a fixed award for a qualifying recording | No award; no employee penalty |
| Employer Match | The employee's explicitly accepted company-point pledge is returned and the company bonus is added | After the upload/review deadlines, closure moves the pledge into the company pool |
| Monthly Budget | A provisional award declines linearly through the agreed interval; a qualifying run earns the remaining amount | Only the unearned allocation expires; already earned employee points remain untouched |

The prototype verifies running GPX/FIT files. Swimming, step-counter input, live GPS leaderboards, health connectors, Cafeteria payouts and cryptographic zero-knowledge proofs are not implemented. The Event template shares the existing evidence workflow; it is not a live race-tracking service.

An Employer Match pledge requires available points already earned in this same company. A new employee can first complete Activity Points or Event programs. No personal LOCAL credits, THT or money are taken. Match and Monthly Budget require an explicit acceptance of their published terms; participation remains voluntary.

Monthly Budget is the prototype's declining *prospective* reward, not a reduction of existing contractual benefits. The amount is calculated at the recorded run's end in LIVE mode and at upload admission in REPLAY mode, then retained with the award. A later review does not change that evaluation time. The available preview uses the server clock. Integer rounding can make the last part of an interval worth zero points. Late uploads cannot claim the earlier maximum merely by delaying review.

**Next monthly cycle** creates a separate next-calendar-month draft with its own identity and dates. The owner must explicitly publish and fund it. There is no background subscription, automatic monthly billing or scheduler that replenishes company money.

## New point-program flow

1. Create a workspace. Its owner tops up the **simulated company pool**, choosing an integer number of points.
2. Create a draft, select one of the four templates, and set the running target, point award and capacity. Match also sets a pledge.
3. Publish **Record a new run** or **Use a saved GPX/FIT** terms; the target, reward and dates then become fixed.
4. Invite participants. Joining voluntarily reserves a funded place. Match locks the accepted company-point pledge; other templates take no employee stake.
5. Participants upload GPX/FIT. The same consistency and eligibility rules apply as B2C.
6. Acceptance automatically awards points in the same database transaction. Evidence requiring review pays only after an authorized acceptance. Retrying an upload or claim does not pay twice.
7. Close the program when server conditions allow. Unused company points return to the company pool; an unresolved Match pledge is settled according to its accepted terms. Archive finished programs/workspaces to remove them from active lists.

The point dashboard shows company available/reserved/awarded totals to managers, and each employee only their own available and pledged points and movements. Issuance is an owner-only simulation action with retry protection. Points are not exchangeable for CZK in this prototype and there is no payment or tax-exemption claim.

## Existing LOCAL programs

Programs created before the point templates remain `LEGACY`. Their original behavior is preserved: the owner's personal LOCAL credits fund the complete published budget, accepted evidence permits an explicit reward claim, and unused budget returns to the original funder. Rewards remain 1–50 LOCAL credits, with at most 1,000 credits per program. Legacy draft CZK planning amounts are not actual payments or a conversion rate.

## Activity and privacy

**Accepted** means the distance and applicable window were met without a review-triggering consistency finding. **Not accepted** means the recording did not qualify for this target/window. **Review required** needs an authorized operator decision; company management roles alone are insufficient. See [Activity verification](ACTIVITY_VERIFICATION.md).

Participants and authorized reviewers can inspect the private recording and checks. Employers see participation and result/reward status rather than another participant's GPS, source file or biometric aggregates.

File hashes and a normalized fingerprint prevent qualifying recordings from being credited repeatedly for the same user across B2C and B2B. These controls cannot authenticate an editable file or detect every fabrication.

## Closure, archive and deletion

A published program can close after the upload deadline, before its start with no participants, or earlier after the full capacity has been rewarded. An accepted unpaid reward blocks closure and does not expire. A pending review remains protected until the review deadline. After that deadline, unaccepted participation can close; Match's explicitly accepted pledge rule then applies.

The detail page explains current closure availability, waiting rewards/reviews and the relevant deadline. One participant in a program with two places does not justify early closure while uploads remain open. Refreshing status updates availability; it does not close or pay automatically.

Draft and closed programs can be archived. A workspace can be archived by its owner after its published programs and budget are settled. Archiving preserves members and history and makes the workspace read-only until restoration.

Permanent deletion requires the owner, exact workspace name, current password, no other members and no previously funded program or corporate point history. A workspace with financial history must use closure and archiving instead. The UI checks eligibility before offering the deletion form. Developer profile reset protects corporate point history shared with other accounts; see [Development](DEVELOPMENT.md).

## Limits and integrity

New point awards and Match pledges range from 1–100,000 integer points, with at most 10,000,000 points funded per program. Capacity, available company points and employee pledges are checked inside database transactions. Account and organization locks, program/enrollment locks and unique constraints prevent overspending and duplicate rewards. Every corporate point movement must balance to zero, including its issuance offset.

LIVE programs require future-start terms and an activity entirely within the agreed window. Monthly programs support intervals up to 32 days; other programs support up to 30 days. REPLAY permits older recordings for demonstrating the workflow; the server sets a short upload/review period. Both reject future recordings. Uploads are limited to 16 MiB and 20 saved recordings per enrollment; FIT session selection is supported.

Request identifiers do not grant permissions. Changes require authentication, applicable organization/operator permission, origin checks and CSRF protection. State/version checks run again under database locks; a button being displayed is not sufficient authorization.

Tests in `crates/api/tests/business.rs` preserve legacy behavior; `business_points.rs` covers the independent point pool, templates, automatic awards, pledges, declining awards, privacy and retries. Developer reset tests protect company history. Creating a test is not evidence of a passed run; execute the relevant verification commands.

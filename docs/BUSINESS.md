# Team programs

B2B uses the same account system as personal challenges, with separate organization memberships and program participation. Its financial flow is entirely **LOCAL test credits**: no real payment, company verification, Devnet transfer or participant Phantom connection.

## Roles and invitation

| Role | Responsibilities |
| --- | --- |
| Owner | Workspace ownership, administrator roles, ownership transfer and program funding |
| Administrator | Member invitations, draft management and permitted program operations |
| Member | Voluntary participation and access to their own activity/result |
| Prototype operator | Explicit separate review permission; can inspect private evidence and resolve eligible reviews or reward claims |

Invitations are delivered manually. The server returns the secret token only on first creation, stores its hash and accepts it only from an account matching the invited email. Tokens expire after seven days and can be revoked. No invitation email is sent automatically. Reviewer provisioning is in [Development](DEVELOPMENT.md).

## Program flow

1. Create a workspace and a program draft with the running target and capacity.
2. Add LOCAL test credits to the owner's balance. Publication specifies a separate LOCAL reward and reserves the full maximum reward budget.
3. Publish **Record a new run** or **Use a saved GPX/FIT** terms; the target, reward and dates then become fixed.
4. Invite participants. Joining voluntarily reserves a funded place; participants do not pay a stake or lose personal credits if they do not qualify.
5. Participants upload GPX/FIT. The same consistency and eligibility rules apply as B2C.
6. An accepted result allows **Claim reward**. The participant or authorized operator triggers one LOCAL transfer, atomically reducing the funding reserve and crediting the participant.
7. Close the program when server conditions allow; unused budget returns to the original funder, including after workspace ownership changes. Archive finished programs/workspaces to remove them from active lists.

Draft planning amounts in CZK are not actual payments and are not converted into LOCAL credits. Publication explicitly sets the LOCAL reward separately. A published program does not use the personal B2C escrow.

## Activity and privacy

**Accepted** means the distance and applicable window were met without a review-triggering consistency finding. **Not accepted** means the recording did not qualify for this target/window. **Review required** needs an authorized operator decision; company management roles alone are insufficient. See [Activity verification](ACTIVITY_VERIFICATION.md).

Participants and authorized reviewers can inspect the private recording and checks. Employers see participation and result/reward status rather than another participant's GPS, source file or biometric aggregates.

File hashes and a normalized fingerprint prevent qualifying recordings from being credited repeatedly for the same user across B2C and B2B. These controls cannot authenticate an editable file or detect every fabrication.

## Closure, archive and deletion

A published program can close after the upload deadline, before its start with no participants, or earlier after the full capacity has been rewarded. An accepted unpaid reward blocks closure and does not expire. A pending review remains protected until the review deadline. After that deadline, unaccepted participation can close without a reward or participant penalty.

The detail page explains current closure availability, waiting rewards/reviews and the relevant deadline. One participant in a program with two places does not justify early closure while uploads remain open. Refreshing status updates availability; it does not close or pay automatically.

Draft and closed programs can be archived. A workspace can be archived by its owner after its published programs and budget are settled. Archiving preserves members and history and makes the workspace read-only until restoration.

Permanent deletion requires the owner, exact workspace name, current password, no other members and no previously funded program. A workspace with funded history must use closure and archiving instead. The UI checks eligibility before offering the deletion form. Developer profile reset has separate protected-data rules; see [Development](DEVELOPMENT.md).

## Limits and integrity

Rewards range from 1–50 LOCAL credits, with at most 1,000 credits reserved per program. Capacity and funding are checked inside database transactions. Account locks, program/enrollment locks and unique constraints prevent overspending and duplicate rewards.

LIVE programs require future-start terms and an activity entirely within the agreed window. REPLAY permits older recordings for demonstrating the workflow; the server sets a short upload/review period. Both reject future recordings. Uploads are limited to 16 MiB and 20 saved recordings per enrollment; FIT session selection is supported.

Request identifiers do not grant permissions. Changes require authentication, applicable organization/operator permission, origin checks and CSRF protection. State/version checks run again under database locks; a button being displayed is not sufficient authorization.

Tests in `crates/api/tests/business.rs` cover roles, invitations, funding/capacity races, rewards, returns, privacy, replay protection, deadlines and lifecycle behavior. Creating a test is not evidence of a passed run; execute the relevant verification commands.
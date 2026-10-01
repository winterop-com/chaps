//! The notes, hints and refusals the component commands print.

use super::*;

/// The note `components enable ocs` prints while there is no object store.
///
/// OCS does not read any S3 variable yet, so this is a heads-up rather than a
/// requirement: forcing a second container on a deployment for a contract that
/// does not exist would be worse than saying it is coming.
pub const S3_SOON_NOTE: &str = "OCS will soon need an S3-compatible object store; `chaps components enable s3` \
     adds one, and the OCS service then gets the S3_* variables it will read";

/// The note `components enable s3` prints on a deployment with no OCS.
///
/// The store is there for OCS to keep its objects in, and nothing else in a
/// chaps deployment writes to it - so a deployment that has one and no OCS runs
/// a container with nothing to put in it. The mirror image of
/// [`S3_SOON_NOTE`], and a note for the same reason: it is a container the
/// operator may well be adding first on purpose.
pub const S3_WITHOUT_OCS_NOTE: &str = "the object store is for OCS to keep its objects in, and this deployment has no OCS; \
     `chaps components enable ocs` adds one, and nothing else here writes to the store";

/// The note `components disable s3` prints while OCS is still enabled.
///
/// The next sync re-renders `compose.ocs.yml` without the `S3_*` block, which
/// is a change to a service the operator did not name. OCS does not read those
/// variables yet, so this says what went rather than warning about a breakage
/// there is not.
pub const S3_LEAVES_OCS_NOTE: &str = "the OCS service loses its S3_* variables on this sync; OCS does not read them yet, \
     and `chaps components enable s3` puts them back";

/// The note `components enable ocs` prints on the run that puts the data source
/// variables into `.env`.
///
/// `chaps sync` appends them commented out, and until now the only sign of it
/// was `written .env`. Which datasets need which key is the thing an operator
/// cannot guess from the variable names, so the line says that much and names
/// the command that reports what is set. "One or both" is as much of it as fits
/// on a line: the store alone, the hub alone and both together are each some
/// ERA5-Land dataset's answer, and `docs/components.md` says which is which.
pub const OCS_DATA_SOURCE_NOTE: &str = "the OCS data source variables are now in `.env`, commented out: ERA5-Land needs \
     one or both of ECMWF_DATASTORES_* and EDH_API_KEY, per dataset; WorldPop and \
     CHIRPS3 need none. `chaps auth show` reports which are set";

/// The note `components enable dhis2` and `init --with dhis2` print about what
/// the first start will do with the database.
///
/// The restore is the postgres entrypoint's own, so it happens once, on a data
/// directory it has just created, and never again - which is the half an
/// operator cannot see from the compose file and the half that decides whether
/// changing the seed later does anything. So the line says "once" and names the
/// command that makes the volume fresh again.
pub fn dhis2_seed_note(seed: Option<&str>) -> String {
    match seed {
        Some(source) => format!(
            "the first `chaps up` restores {source} into `dhis2_db`, once, on the database it \
             creates; after that only `chaps components disable dhis2 --purge` makes it happen again"
        ),
        None => "`dhis2_db` starts empty and DHIS2 migrates a new database into it; `seed:` in \
             `.chaps/components.yaml` names a dump instead, then run `chaps sync`"
            .to_string(),
    }
}

/// The note printed when the seed was left at `default` and the pinned minor
/// line publishes no dump chaps knows of.
///
/// Said rather than guessed: the published path does not follow from the tag
/// (see [`DHIS2_SEED_DUMPS`]), so constructing one would download a 404 on the
/// first start and leave the operator with an empty DHIS2 and no explanation.
pub fn dhis2_unknown_seed(tag: &str) -> String {
    // A version is named as one; a tag such as `master` is a build, and
    // "DHIS2 master" would read as a release that has no dump yet.
    let what = match tag.trim().starts_with(|c: char| c.is_ascii_digit()) {
        true => dhis2_minor(tag).to_string(),
        false => format!("the image tag `{}`", tag.trim()),
    };
    format!(
        "chaps knows no DHIS2 demo dump for {what}, so `dhis2_db` starts empty; name one with \
         `seed:` in `.chaps/components.yaml` (a URL or a path) and run `chaps sync`"
    )
}

/// The note about how long the first start takes.
///
/// Every other service in a deployment answers in seconds; DHIS2 migrates its
/// whole schema before it serves a request, and under emulation that is a
/// quarter of an hour. An operator who does not know that reads the first
/// `chaps status` as a broken deployment.
pub const DHIS2_FIRST_START_NOTE: &str = "the first `chaps up` takes minutes before DHIS2 answers - it migrates its schema on the \
     way up - and `chaps logs dhis2` is where that shows";

/// The note that says the two halves are not connected yet, and what connects
/// them.
///
/// A DHIS2 and a Chap started side by side cannot talk: the Modeling App
/// reaches chap-core through a DHIS2 route, which this deployment does not have
/// until something creates it. `chaps up` does not, on purpose - it is a thin
/// wrapper around compose, DHIS2's API is not ready when it returns, and the
/// steps need DHIS2 credentials and the network - so the command is named here,
/// where the component is added and the reader is already being told what the
/// first start does.
pub const DHIS2_CONNECT_NOTE: &str = "the Modeling App reaches chap-core through a DHIS2 route, and this deployment has none \
     yet; once DHIS2 answers, `chaps dhis2 connect` adds it, generates analytics and installs \
     the apps";

/// The line `chaps up` and `chaps status` close with while
/// [`Components::dhis2_needs_connecting`] holds.
///
/// [`DHIS2_CONNECT_NOTE`] is said where the component is added, which is before
/// DHIS2 exists: the reader then runs `chaps up`, waits out a restore and a
/// migration, and lands on a deployment where every row says `up` with that one
/// line minutes above them. So the two commands that report on a running
/// deployment say it again, and go on saying it until a connect is recorded.
///
/// It claims only what it knows. `chaps` has not connected this DHIS2 - that is
/// [`Dhis2Component::connected_at`], read off `.chaps/components.yaml` - rather
/// than "DHIS2 is not connected", which would be a claim about an instance
/// nothing here asked. Naming the command is safe either way, because every
/// `chaps dhis2` verb is idempotent.
///
/// `answering` is whether the caller has just seen DHIS2 answer. `chaps status`
/// has asked `/api/ping` and only says this when it answered; `chaps up` has
/// asked nothing and DHIS2 is minutes from its first request, so it says when.
pub fn dhis2_connect_hint(answering: bool) -> String {
    match answering {
        true => DHIS2_NOT_CONNECTED.to_string(),
        false => format!("{DHIS2_NOT_CONNECTED} once DHIS2 answers"),
    }
}

/// Why the `dhis2` component cannot go on while an external DHIS2 is recorded.
///
/// Both would be "this deployment's DHIS2", and `chaps dhis2` would have to
/// pick one without saying so.
pub fn dhis2_external_refusal(url: &str) -> String {
    format!(
        "this deployment already uses the external DHIS2 at {url}; run `chaps dhis2 use --clear` \
         first to deploy one of its own"
    )
}

/// The half of [`dhis2_connect_hint`] both shapes share.
const DHIS2_NOT_CONNECTED: &str =
    "chaps has not connected this DHIS2 to Chap; run `chaps dhis2 connect`";

/// The note `chaps components disable dhis2` prints when it has just forgotten
/// a recorded connect.
///
/// Said only on the run that forgets something: a deployment that was never
/// connected has nothing to report, and the line would be noise on every
/// disable. See [`Dhis2Component::connected_at`] for why it is forgotten at
/// all.
pub const DHIS2_CONNECT_FORGOTTEN: &str = "the record of `chaps dhis2 connect` is forgotten with the component; a DHIS2 enabled \
     here again is asked to connect afresh";

/// The note `chaps down --volumes` prints when it has just removed the database
/// a recorded connect was true of.
///
/// The worst shape a stale record could make: `dhis2_db` is gone, the next
/// `chaps up` restores the seed dump into a new one, and that dump ships its own
/// `chap` route pointing at a server this deployment has nothing to do with. A
/// record that survived would suppress the one line that asks the operator to
/// repoint it.
pub const DHIS2_CONNECT_FORGOTTEN_WITH_VOLUME: &str = "the record of `chaps dhis2 connect` went with `dhis2_db`; the next `chaps up` restores \
     the seed dump, which ships a `chap` route of its own, so run `chaps dhis2 connect` again";

/// The same for a DHIS2 with no seed: the next `chaps up` migrates an empty
/// database, which has no route at all.
pub const DHIS2_CONNECT_FORGOTTEN_UNSEEDED: &str = "the record of `chaps dhis2 connect` went with `dhis2_db`; the next `chaps up` starts \
     an empty DHIS2 with no `chap` route, so run `chaps dhis2 connect` again once it answers";

/// The warning for a DHIS2 image tag that is moving while the database volume
/// is already there.
///
/// DHIS2 migrates a schema forward only. Starting an older image on a database a
/// newer one has migrated gives an instance that passes its own health check
/// while every API request answers 404, which is the worst shape a deployment
/// can be in: up, and wrong. So the line names `chaps backup` before anything
/// else.
pub fn dhis2_tag_change_note(from: &str, to: &str) -> String {
    format!(
        "the DHIS2 image moves from {from} to {to} and `dhis2_db` is already there: DHIS2 \
         migrates a schema forward only, so run `chaps backup` first - an older image on a \
         migrated database answers healthy while every API request 404s"
    )
}

/// The refusal of a command that only chap-core can answer, on a deployment
/// chap-core is not a component of. `what` names the command, in backticks.
pub fn needs_chap_core(what: &str) -> String {
    format!(
        "{what} needs chap-core, and this deployment has no chap-core; \
         `chaps components enable chap-core` adds one, or \
         `chaps components enable chap-core --url URL` names one elsewhere"
    )
}

/// Refuse `what` when `components` leaves chap-core out and names none
/// elsewhere.
pub fn require_chap_core(components: &Components, what: &str) -> Result<()> {
    if components.has_chap_core_api() {
        return Ok(());
    }
    Err(anyhow::anyhow!(needs_chap_core(what)))
}

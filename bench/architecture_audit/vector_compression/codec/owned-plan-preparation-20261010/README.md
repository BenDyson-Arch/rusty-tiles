This is a frozen **static preparation**, with no author target execution. Read [representation.md](representation.md), [owned_plan_probe.rs](owned_plan_probe.rs), [fixtures.json](fixtures.json), [commands.json](commands.json) and [check_records.py](check_records.py). `input-pin.json` binds immutable original fixtures, accepted C1 helper and exact compiler/dependency receipt; `integrity.json` binds all new package bytes.

Independent review must approve this specific source/probe scope before root runs:

```
python3 /tmp/rusty-tiles-codec-owned-plan-preparation/root_runner.py --out-dir /tmp/ROOT_CHOSEN_NEW_DIRECTORY
```

The runner verifies pins and the accepted C1 gate, uses the frozen existing coordinator nice10/two-core/timeout helper, records compile and52 commands separately, then runs the independent event/owner checker. It never edits production or frozen inputs. Any source correction requires an additive new package/epoch, not rewriting this freeze. `prepare_inputs.py` records fixture authorship; do not rerun it against this freeze. The event checker is also unexecuted by the author. Whole codec acceptance remains held.

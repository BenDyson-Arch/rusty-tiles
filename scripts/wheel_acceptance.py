"""Completion evidence shared by the installed-wheel acceptance runners."""
import json
import shutil


def clear_requested_report(path, wheel):
    if path is None:
        return None
    destination = path.resolve()
    if destination == wheel.resolve():
        raise ValueError("The report path must differ from the input wheel")
    destination.unlink(missing_ok=True)
    return destination


def require_completion(evidence, wheel_sha256, *, require_blender=False):
    if not evidence.is_file():
        raise RuntimeError("The API suite produced no completion evidence")
    report = json.loads(evidence.read_text())
    count = report.get("tests_run")
    if (report.get("ok") is not True or type(count) is not int or count <= 0
            or report.get("wheel_sha256") != wheel_sha256):
        raise RuntimeError("The API suite did not confirm successful acceptance of this wheel")
    if require_blender and not (isinstance(report.get("blender"), str) and report["blender"]):
        raise RuntimeError("The API suite did not provide Blender runtime identity")


def copy_evidence(evidence, destination):
    # Preserve current failure evidence when the suite wrote it, but never copy
    # a report left over from an earlier invocation.
    if destination is not None and evidence.is_file():
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(evidence, destination)

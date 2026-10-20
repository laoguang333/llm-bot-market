"""Apply the already frozen selection rule after both matrix batches finish."""
import json
import shutil
from pathlib import Path

from swift_capital import BASE, ROOT, prepare, run
from swift_capital_report import build, write_markdown
from swift_stress import sha


def main():
    matrix = build(["matrix-a", "matrix-b"])
    assert matrix["runs"] == 1420
    path = ROOT / "docs/swift-capital-results.json"
    path.write_text(json.dumps(matrix, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    write_markdown(matrix, ROOT / "docs/swift-capital-results.md")
    selected = matrix["confirmation_selection"]
    cases = set(selected)
    for key in selected:
        _, allocation, multiple = key.split("-")
        cases.update(f"{mode}-{allocation}-{multiple}" for mode in ["cash", "blind"])
    selection = {"matrix_sha256": sha(path), "selection_script_sha256": sha(Path(__file__)),
                 "protocol_sha256": sha(ROOT / "docs/swift-capital-protocol.md"),
                 "selected": selected, "cases_with_controls": sorted(cases),
                 "rule": "Mean predator-minus-Swift > 0 OR mean Swift < 0; top three by mean advantage; no retuning."}
    (ROOT / "docs/swift-capital-selection.json").write_text(json.dumps(selection, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    print("FROZEN SELECTION", selected, flush=True)
    if selected:
        prepare("confirmation", 16001, 20, sorted(cases))
        shutil.copy2(BASE / "confirmation/plan.json", ROOT / "docs/swift-capital-confirmation-plan.json")
        run("confirmation")


if __name__ == "__main__":
    main()

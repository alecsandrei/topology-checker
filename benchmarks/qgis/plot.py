"""Publish per-rule medians, IQRs and a correctness-gated horizontal chart."""

import argparse
import json
import statistics
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
from matplotlib.ticker import FuncFormatter, LogLocator  # noqa: E402

COLORS = {"rust": "#2563eb", "qgis": "#b45309"}


def quantile(values: list[float], fraction: float) -> float:
    """Interpolate sorted samples using the standard linear quantile convention."""
    if not values:
        raise ValueError("no samples")
    if not 0 <= fraction <= 1:
        raise ValueError("fraction must be between zero and one")
    ordered = sorted(values)
    index = (len(ordered) - 1) * fraction
    lower = int(index)
    upper = min(lower + 1, len(ordered) - 1)
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (index - lower)


def summary(samples: list[dict]) -> dict | None:
    """Summarize successful samples only; never treat failures as timings."""
    seconds = [s["seconds"] for s in samples if s["status"] == "ok"]
    if not seconds:
        return None
    return {
        "n": len(seconds),
        "median_s": statistics.median(seconds),
        "q1_s": quantile(seconds, 0.25),
        "q3_s": quantile(seconds, 0.75),
    }


def label(rule: str) -> str:
    """Give readable but unambiguous rule labels at README display size."""
    names = {
        "point/must-not-overlap": "Duplicate points",
        "point/must-be-inside": "Points inside polygons",
        "line/must-not-have-dangles": "Line dangles",
        "line/must-not-have-pseudo-nodes": "Line pseudo-nodes",
        "polygon/must-not-overlap": "Polygon overlap",
        "polygon/must-not-overlap-with": "Cross-layer polygon overlap",
        "polygon/must-not-have-gaps": "Polygon gaps",
        "geometry/must-not-be-multipart": "Multipart geometries",
        "point/must-be-covered-by-lines": "Points covered by lines",
        "point/must-be-covered-by-line-endpoints": "Points on line endpoints",
        "point/must-be-covered-by-polygon-boundaries": "Points on polygon boundaries",
        "line/endpoints-must-be-covered-by-points": "Line endpoints covered by points",
        "polygon/must-contain-point": "Polygons contain points",
    }
    return names.get(rule, rule)


def draw(data: dict, output: Path, size: int = 10000) -> None:
    """Plot only fully measured pairs whose probes and workload counts agree."""
    paired = [
        (r, w)
        for r in data["rules"]
        for w in r["workloads"]
        if w["size"] == size and w["paired_eligible"]
    ]
    plt.rcParams.update(
        {
            "font.family": "DejaVu Sans",
            "font.size": 11,
            "svg.fonttype": "none",
            "svg.hashsalt": "topology-qgis",
        }
    )
    figure, axis = plt.subplots(figsize=(10, max(2.5, len(paired) * 0.55 + 0.7)))
    for row, (_, work) in enumerate(paired):
        for engine, offset in (("rust", -0.12), ("qgis", 0.12)):
            stats = summary(work[engine])
            if stats is None:
                raise ValueError("Eligible pair has no successful samples")
            median = stats["median_s"] * 1000
            low, high = stats["q1_s"] * 1000, stats["q3_s"] * 1000
            axis.errorbar(
                median,
                row + offset,
                xerr=[[median - low], [high - median]],
                fmt="o",
                markersize=6,
                capsize=3,
                color=COLORS[engine],
                linewidth=1.8,
                label=("Rust (1 thread)" if engine == "rust" else "QGIS 4.2.2")
                if row == 0
                else None,
            )
    axis.set_yticks(range(len(paired)), [label(r["rule"]) for r, _ in paired])
    axis.invert_yaxis()
    axis.set_xscale("log")
    axis.xaxis.set_major_locator(LogLocator(base=10, subs=(1, 2, 5)))
    axis.xaxis.set_major_formatter(FuncFormatter(lambda value, _: f"{value:g}"))
    axis.set_xlabel("Validation runtime (ms, log scale)", labelpad=10)
    axis.set_ylabel("Topology rule", labelpad=12)
    axis.grid(axis="x", which="major", alpha=0.2)
    axis.set_axisbelow(True)
    for side in ("top", "right", "left"):
        axis.spines[side].set_visible(False)
    axis.tick_params(axis="y", length=0, pad=10)
    if paired:
        axis.legend(
            loc="lower left",
            bbox_to_anchor=(0, 1.02),
            ncol=2,
            frameon=False,
            borderaxespad=0,
            fontsize=10,
        )
    figure.tight_layout()
    figure.savefig(output / "comparison.svg", metadata={"Date": None})
    figure.savefig(output / "comparison.png", dpi=130)
    plt.close(figure)


def publish(path: Path, output: Path) -> None:
    """Generate the chart, complete mapping and results table from saved JSON."""
    data = json.loads(path.read_text())
    output.mkdir(parents=True, exist_ok=True)
    draw(data, output)
    lines = [
        "# Measured validation results",
        "",
        f"Run: {data['metadata']['started_at']}. Raw samples, hashes and machine details: [results.json](results.json).",
        "",
        "Times are medians in milliseconds, with Q1 and Q3 in parentheses. Counts are raw result records from the first successful measured sample. Failed or incomplete runs are never paired.",
        "",
        "| Rule | Primary features | Secondary features | Rust ms (Q1, Q3) | QGIS ms (Q1, Q3) | Raw counts (Rust / QGIS) | Comparison |",
        "| :--- | ---: | ---: | ---: | ---: | :--- | :--- |",
    ]
    for record in data["rules"]:
        mismatches = [p["name"] for p in record["probes"] if p.get("agrees") is False]
        for work in record["workloads"]:
            values = []
            counts = []
            for engine in ("rust", "qgis"):
                stats = summary(work[engine])
                values.append(
                    f"{stats['median_s'] * 1000:.3f} ({stats['q1_s'] * 1000:.3f}, {stats['q3_s'] * 1000:.3f})"
                    if stats
                    else "N/A"
                )
                samples = [s for s in work[engine] if s["status"] == "ok"]
                counts.append(str(samples[0]["raw_count"]) if samples else "N/A")
            if record["mapping"] is None:
                status = "No plugin counterpart"
            elif work["paired_eligible"]:
                status = "Paired, probes passed"
            elif mismatches:
                status = "Unpaired, probes differ: " + ", ".join(mismatches)
            else:
                status = (
                    "Unpaired, workload counts differ"
                    if not work["workload_agrees"]
                    else "Unpaired, failure or incomplete samples"
                )
            if work["failures"]:
                status += "; " + ", ".join(
                    f"{e}: {v['status']} ({v['error'].splitlines()[-1]})"
                    for e, v in work["failures"].items()
                )
            secondary = work["fixture"].get("other", {}).get("features", 0)
            lines.append(
                f"| `{record['rule']}` | {work['size']:,} | {secondary:,} | {values[0]} | {values[1]} | {' / '.join(counts)} | {status.replace('|', '/')} |"
            )
    lines += [
        "",
        "## Correctness probes",
        "",
        "The raw probe records retain feature IDs and error kinds. Normalization compares visible primary IDs and counts, collapses endpoint rows to affected feature IDs, and excludes Rust outer boundaries from gap checks. Secondary IDs, exact error locations, invalid-geometry repairs and nonzero tolerances are outside this comparison.",
        "",
        "| Rule | Case | Rust raw count | QGIS raw count | Rust expectation | Normalized comparison |",
        "| :--- | :--- | ---: | ---: | :--- | :--- |",
    ]
    for record in data["rules"]:
        for probe in record["probes"]:
            agreement = (
                "No counterpart"
                if probe["agrees"] is None
                else ("Agree" if probe["agrees"] else "Differ")
            )
            lines.append(
                f"| `{record['rule']}` | {probe['name']} | {probe['rust'].get('raw_count', 'N/A')} | {probe.get('qgis', {}).get('raw_count', 'N/A')} | {'Pass' if probe['rust_expected'] else 'FAIL'} | {agreement} |"
            )
    (output / "README.md").write_text("\n".join(lines) + "\n")
    mapping = [
        "# Complete rule mapping",
        "",
        "These names refer to the installed C++ Topology Checker, not Processing algorithms. An available related check does not imply general equivalence. All 38 project rules are measured on synthetic fixtures.",
        "",
        "| Project rule | Topology Checker rule | Semantics and limits |",
        "| :--- | :--- | :--- |",
    ]
    for record in data["rules"]:
        value = record["mapping"]
        mapping.append(
            f"| `{record['rule']}` | {value[0] if value else 'None'} | {value[1] if value else 'No counterpart in this plugin. Rust-only measurement.'} |"
        )
    (output / "mapping.md").write_text("\n".join(mapping) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("results", type=Path)
    parser.add_argument("--output", type=Path)
    arguments = parser.parse_args()
    publish(arguments.results, arguments.output or arguments.results.parent)

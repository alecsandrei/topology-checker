"""Render the README comparison and full audit tables from saved measurements."""

import csv
import json
import math
import statistics
from pathlib import Path
from typing import Any

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

from .catalog import CHECK_LABELS, ENGINES, HEADLINE_RULES, LABELS

COLORS = {
    "rust": "#2563eb",
    "qgis": "#b45309",
    "grass": "#15803d",
    "postgis": "#7c3aed",
}
# Check and tool pairs the figure requires, topology-checker included.
COMBINATIONS = len(HEADLINE_RULES) * (len(ENGINES) + 1)


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


def summary(samples: list[dict[str, Any]]) -> dict[str, Any] | None:
    """Summarize positive finite measurements without converting failures to zero."""
    values = [s["seconds"] for s in samples if s["status"] == "ok"]
    if any(not math.isfinite(value) or value <= 0 for value in values):
        raise ValueError("timings must be positive and finite")
    if not values:
        return None
    return {
        "n": len(values),
        "median_s": statistics.median(values),
        "q1_s": quantile(values, 0.25),
        "q3_s": quantile(values, 0.75),
    }


def complete_workload(work: dict[str, Any], repetitions: int, warmups: int) -> bool:
    """Require full valid samples, warmups and identical recorded result hashes."""
    engines = ("rust", *ENGINES)
    samples = work.get("samples", {})
    if not all(len(samples.get(name, [])) == repetitions for name in engines):
        return False
    preparation = work.get("warmups", [])
    if not all(
        sum(s.get("engine") == name for s in preparation) == warmups for name in engines
    ):
        return False
    values = [s for name in engines for s in samples[name]] + preparation
    if any(
        s.get("status") != "ok"
        or not math.isfinite(s.get("seconds", 0))
        or s.get("seconds", 0) <= 0
        for s in values
    ):
        return False
    hashes = {s.get("result_sha256") for s in values}
    return len(hashes) == 1 and None not in hashes


def headline_size(data: dict[str, Any]) -> int:
    """Choose a size only when every check and tool has complete measurements."""
    paired_sizes = []
    repetitions = data["metadata"].get("repetitions", 10)
    for rule in HEADLINE_RULES:
        work = [w for r in data["rules"] if r["rule"] == rule for w in r["workloads"]]
        paired_sizes.append(
            {
                w["size"]
                for w in work
                if w["size"] >= 1000
                and all(
                    w["comparisons"].get(name, {}).get("eligible", False)
                    for name in ENGINES
                )
                and complete_workload(
                    w, repetitions, data["metadata"].get("warmups", 2)
                )
            }
        )
    return max(set.intersection(*paired_sizes), default=0)


def headline_workload(data: dict[str, Any], rule: str) -> dict[str, Any] | None:
    """Use one common feature count across all six checks and all tools."""
    size = headline_size(data)
    return next(
        (
            w
            for r in data["rules"]
            if r["rule"] == rule
            for w in r["workloads"]
            if w["size"] == size
        ),
        None,
    )


def publishable(data: dict[str, Any]) -> bool:
    """Require all six checks and full provenance from native, single-CPU Linux CI."""
    metadata = data["metadata"]
    execution = metadata.get("execution", {})
    return bool(
        headline_size(data)
        and execution.get("native_linux_ci")
        and len(execution.get("cpu_affinity") or []) == 1
        and metadata.get("source_revision")
        and metadata.get("source_tree_sha256")
        and metadata.get("tooling_sha256")
        and metadata.get("container_image")
        and metadata.get("rust_binary_sha256")
        and metadata.get("packages")
        and metadata.get("repetitions") == 10
        and metadata.get("warmups") == 2
        and all(
            data["engines"].get(name, {}).get("version")
            or (name == "rust" and data["engines"].get(name, {}).get("package_version"))
            for name in ("rust", *ENGINES)
        )
    )


def draw(data: dict[str, Any], output: Path) -> None:
    """Draw one minimal grouped bar chart only after every comparison passes."""
    for filename in ("comparison.png", "comparison.svg"):
        (output / filename).unlink(missing_ok=True)
    if not headline_size(data):
        return
    engines = ["rust", *ENGINES]
    plt.rcParams.update(
        {
            "font.family": "DejaVu Sans",
            "font.size": 10,
            "svg.fonttype": "none",
            "svg.hashsalt": "topology-gis",
        }
    )
    figure, axis = plt.subplots(figsize=(11, 4))
    width = 0.75 / len(engines)
    for offset, engine in enumerate(engines):
        values = []
        for rule in HEADLINE_RULES:
            work = headline_workload(data, rule)
            assert work is not None
            stats = summary(work["samples"][engine])
            assert stats is not None
            values.append(stats["median_s"] * 1000)
        axis.bar(
            [x + (offset - (len(engines) - 1) / 2) * width for x in range(6)],
            values,
            width=width,
            color=COLORS[engine],
            label=LABELS[engine],
        )
    axis.set_xticks(range(6), CHECK_LABELS)
    axis.set_yscale("log")
    axis.set_ylabel("Median validation time (ms)")
    axis.legend(
        ncols=len(engines),
        loc="upper center",
        bbox_to_anchor=(0.5, 1.18),
        frameon=False,
    )
    axis.grid(False)
    for side in ("top", "right"):
        axis.spines[side].set_visible(False)
    figure.tight_layout()
    svg = output / "comparison.svg"
    figure.savefig(svg, metadata={"Date": None})
    svg.write_text(
        "\n".join(line.rstrip() for line in svg.read_text().splitlines()) + "\n"
    )
    figure.savefig(output / "comparison.png", dpi=160)
    plt.close(figure)


def publish(data: dict[str, Any], output: Path) -> None:
    """Export a figure, raw samples CSV, probe details and per-engine comparisons."""
    output.mkdir(parents=True, exist_ok=True)
    draw(data, output)
    lines = [
        "# Native Linux GIS validation benchmarks",
        "",
        "[Raw measurements](results.json) · [Samples CSV](samples.csv) · [SVG figure](comparison.svg)",
        "",
        "![Six checks](comparison.png)"
        if headline_size(data)
        else f"Figure withheld: all {COMBINATIONS} comparisons must pass at one common size.",
        "",
        "Times include fresh index or topology construction and materialized violations. "
        "Application startup, input loading, fresh copies and response extraction are excluded. "
        "PostGIS is measured as SQL workflows; GRASS module launch and native output writing "
        "are included. Comparisons require passing probes and matching workload provenance.",
        "",
        "| Rule | Features | Engine | Median ms | Q1 ms | Q3 ms | Samples | Tool / ours | Status |",
        "| :--- | ---: | :--- | ---: | ---: | ---: | ---: | ---: | :--- |",
    ]
    csv_rows: list[dict[str, Any]] = []
    for record in data["rules"]:
        for work in record["workloads"]:
            baseline = summary(work["samples"].get("rust", []))
            for engine in dict.fromkeys(("rust", *data["engines"])):
                samples = work["samples"].get(engine, [])
                stats = summary(samples)
                comparison = work["comparisons"].get(engine, {})
                status = (
                    "baseline"
                    if engine == "rust"
                    else comparison.get("reason", "unsupported")
                )
                ratio = (
                    stats["median_s"] / baseline["median_s"]
                    if stats and baseline and comparison.get("eligible")
                    else None
                )
                values = [
                    f"{stats[key] * 1000:.3f}" if stats else "N/A"
                    for key in ("median_s", "q1_s", "q3_s")
                ]
                lines.append(
                    f"| `{record['rule']}` | {work['size']:,} | {LABELS[engine]} | "
                    + " | ".join(values)
                    + f" | {stats['n'] if stats else 0} | "
                    + (f"{ratio:.2f}×" if ratio is not None else "N/A")
                    + f" | {status} |"
                )
                for sample in samples:
                    csv_rows.append(
                        {
                            "rule": record["rule"],
                            "size": work["size"],
                            "engine": engine,
                            **sample,
                        }
                    )
    lines.extend(
        [
            "",
            "## Correctness probes",
            "",
            "| Rule | Probe | Engine | Result |",
            "| :--- | :--- | :--- | :--- |",
        ]
    )
    for record in data["rules"]:
        for probe in record["probes"]:
            for engine, value in probe["engines"].items():
                status = value.get("reason", value["status"])
                lines.append(
                    f"| `{record['rule']}` | {probe['name']} | {LABELS[engine]} | {status} |"
                )
    lines.extend(["", "## Native operations", ""])
    for record in data["rules"]:
        for engine, operation in record.get("operations", {}).items():
            lines.append(f"- `{record['rule']}`, {LABELS[engine]}: {operation}")
    lines.extend(["", "## Engine availability and versions", ""])
    for engine, metadata in data["engines"].items():
        lines.append(
            f"- {LABELS[engine]}: {metadata.get('status', 'ready')}. "
            f"Version: {metadata.get('version', metadata.get('package_version', 'unavailable'))}. "
            f"{metadata.get('reason', '')}".rstrip()
        )
    lines.extend(
        [
            "",
            "## Provenance",
            "",
            "```json",
            json.dumps(data["metadata"], indent=2),
            "```",
            "",
        ]
    )
    (output / "README.md").write_text("\n".join(lines))
    fields = sorted({key for row in csv_rows for key in row})
    with (output / "samples.csv").open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=fields, lineterminator="\n")
        writer.writeheader()
        writer.writerows(csv_rows)
    marker = output / "publishable.json"
    marker.unlink(missing_ok=True)
    if publishable(data):
        marker.write_text(
            json.dumps(
                {
                    "source_revision": data["metadata"]["source_revision"],
                    "rules": list(HEADLINE_RULES),
                    "size": headline_size(data),
                }
            )
            + "\n"
        )

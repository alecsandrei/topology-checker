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

from benchmarks.qgis.plot import quantile

from .catalog import ENGINES, LABELS, SHARED_RULES

COLORS = {
    "rust": "#2563eb",
    "qgis": "#b45309",
    "grass": "#15803d",
    "postgis": "#7c3aed",
    "gvsig": "#0891b2",
    "fme": "#be185d",
}


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


def headline_size(data: dict[str, Any]) -> int:
    """Choose the largest common size with complete comparisons for every tool."""
    paired_sizes = []
    measured_sizes = []
    for rule in SHARED_RULES:
        work = [w for r in data["rules"] if r["rule"] == rule for w in r["workloads"]]
        measured_sizes.append({w["size"] for w in work if w["size"] >= 1000})
        paired_sizes.append(
            {
                w["size"]
                for w in work
                if w["size"] >= 1000
                and all(
                    w["comparisons"].get(name, {}).get("eligible", False)
                    for name in ENGINES
                )
            }
        )
    paired = set.intersection(*paired_sizes)
    measured = set.intersection(*measured_sizes)
    return max(paired or measured, default=10000)


def headline_workload(data: dict[str, Any], rule: str) -> dict[str, Any] | None:
    """Use one common feature count across both panels and all tools."""
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
    """Require both shared comparisons for every OSS engine on native Linux CI."""
    if not data["metadata"].get("execution", {}).get("native_linux_ci"):
        return False
    for rule in SHARED_RULES:
        work = headline_workload(data, rule)
        if work is None or not all(
            work["comparisons"].get(name, {}).get("eligible", False) for name in ENGINES
        ):
            return False
    return True


def draw(data: dict[str, Any], output: Path) -> None:
    """Keep all tools visible while plotting only correctness-eligible samples."""
    engines = ["rust", *ENGINES]
    if data["engines"].get("fme", {}).get("status") == "ready":
        engines.append("fme")
    plt.rcParams.update(
        {
            "font.family": "DejaVu Sans",
            "font.size": 10,
            "svg.fonttype": "none",
            "svg.hashsalt": "topology-gis",
        }
    )
    figure, axes = plt.subplots(1, 2, figsize=(11, 4), sharey=True)
    titles = ("Points strictly inside polygons", "Polygons contain a point")
    for axis, rule, title in zip(axes, SHARED_RULES, titles, strict=True):
        work = headline_workload(data, rule)
        for row, engine in enumerate(engines):
            comparison = {} if work is None else work["comparisons"].get(engine, {})
            eligible = engine == "rust" or comparison.get("eligible", False)
            samples = [] if work is None else work["samples"].get(engine, [])
            stats = summary(samples) if eligible else None
            if stats:
                median, low, high = (
                    stats[key] * 1000 for key in ("median_s", "q1_s", "q3_s")
                )
                axis.errorbar(
                    median,
                    row,
                    xerr=[[median - low], [high - median]],
                    fmt="o",
                    capsize=4,
                    color=COLORS[engine],
                    markersize=7,
                )
                axis.annotate(
                    f"{median:.2f} ms",
                    (median, row),
                    xytext=(8, 7),
                    textcoords="offset points",
                    fontsize=9,
                )
            else:
                reason = comparison.get("reason", "not measured")
                axis.text(
                    0.03,
                    row,
                    reason,
                    transform=axis.get_yaxis_transform(),
                    va="center",
                    color="#64748b",
                    fontsize=8,
                )
        axis.set_title(title, fontsize=11, pad=13)
        axis.set_xscale("log")
        axis.set_xlabel("Validation time (ms)")
        axis.grid(axis="x", alpha=0.2)
        axis.set_axisbelow(True)
        for side in ("top", "right", "left"):
            axis.spines[side].set_visible(False)
        axis.tick_params(axis="y", length=0)
        axis.margins(x=0.45)
    labels = []
    for engine in engines:
        version = str(data["engines"].get(engine, {}).get("version", "")).split(" ")[0]
        labels.append(LABELS[engine] + (f" {version}" if version else ""))
    axes[0].set_yticks(range(len(engines)), labels)
    axes[0].invert_yaxis()
    figure.suptitle("Same data, same Linux runner, one logical CPU", fontsize=12)
    figure.text(
        0.5,
        0.025,
        f"{headline_size(data):,} primary and {headline_size(data):,} secondary features · median and interquartile range",
        ha="center",
        fontsize=9,
        color="#475569",
    )
    figure.tight_layout(rect=(0, 0.075, 1, 0.94))
    figure.savefig(output / "comparison.svg", metadata={"Date": None})
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
        "![Shared checks](comparison.png)",
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
    lines.extend(["", "## Engine availability", ""])
    for engine, metadata in data["engines"].items():
        lines.append(
            f"- {LABELS[engine]}: {metadata.get('status', 'ready')}. "
            f"{metadata.get('reason', '')}"
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
        writer = csv.DictWriter(stream, fieldnames=fields)
        writer.writeheader()
        writer.writerows(csv_rows)
    marker = output / "publishable.json"
    marker.unlink(missing_ok=True)
    if publishable(data):
        marker.write_text(
            json.dumps(
                {
                    "source_revision": data["metadata"]["source_revision"],
                    "rules": list(SHARED_RULES),
                    "size": headline_size(data),
                }
            )
            + "\n"
        )

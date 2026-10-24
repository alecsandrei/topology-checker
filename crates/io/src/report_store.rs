//! Disk-backed validation rows with fixed-width random-access offsets.
use crate::codec;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
};
use topology_checker_core::{
    FeatureRef, LayerId, ReportSummary, Rule, RuleSummary, Topology, ValidationEvent, Violation,
    ViolationKind,
};

#[derive(Debug, Serialize, Deserialize)]
struct Metadata {
    rule: usize,
    kind: ViolationKind,
    origin: Option<(usize, i64, Option<u32>)>,
    destination: Option<(usize, i64, Option<u32>)>,
}
fn sink_error(error: topology_checker_core::SinkError<anyhow::Error>) -> anyhow::Error {
    match error {
        topology_checker_core::SinkError::Core(e) => anyhow::Error::new(e),
        topology_checker_core::SinkError::Consumer(e) => e,
    }
}
fn reference(r: Option<FeatureRef>) -> Option<(usize, i64, Option<u32>)> {
    r.map(|r| (r.layer.index(), r.fid, r.part))
}
/// Immutable validation metadata and binary rows owned by temporary storage.
#[derive(Debug)]
pub struct DiskReport {
    storage: tempfile::TempDir,
    /// Final rule counts and warnings, without retained violation geometry.
    pub summary: ReportSummary,
    layers: Vec<LayerId>,
}
impl DiskReport {
    /// Validate into workspace-owned temporary binary files, bounded by input and
    /// the core rule's construction scratch rather than retained report rows.
    ///
    /// # Errors
    /// Returns input, validation, encoding or disk-write failures.
    pub fn validate(topology: &Topology, parent: Option<&Path>) -> Result<Self> {
        Self::validate_with_progress(topology, parent, |_, _| Ok(()))
    }
    /// Like [`validate`](Self::validate), calling `rule_end` with each rule's
    /// stored rows as soon as the rule finishes, before the next one starts.
    /// An error from `rule_end` stops validation and is returned.
    ///
    /// # Errors
    /// Returns input, validation, encoding, disk-write or `rule_end` failures.
    pub fn validate_with_progress(
        topology: &Topology,
        parent: Option<&Path>,
        mut rule_end: impl FnMut(&RuleSummary, RuleRows<'_>) -> Result<()>,
    ) -> Result<Self> {
        let mut builder = tempfile::Builder::new();
        builder.prefix("topology-report-");
        let storage = match parent {
            Some(root) => builder.tempdir_in(root)?,
            None => builder.tempdir()?,
        };
        let path = storage.path().join("rows.bin");
        let layers = topology.layers().map(|(id, _)| id).collect::<Vec<_>>();
        let mut rows = BufWriter::new(File::create(&path)?);
        let mut offsets = BufWriter::new(File::create(storage.path().join("offsets.bin"))?);
        let mut offset = 0_u64;
        let mut rule_start = 0_u64;
        let summary = topology
            .validate_events(|event| -> Result<()> {
                let (spec, v) = match event {
                    ValidationEvent::Violation(spec, v) => (spec, v),
                    ValidationEvent::RuleEnd(summary) => {
                        let start = std::mem::replace(&mut rule_start, offset);
                        return rule_end(
                            summary,
                            RuleRows {
                                writer: &mut rows,
                                path: &path,
                                start,
                                summary,
                                layers: &layers,
                                left: summary.total,
                                input: None,
                            },
                        );
                    }
                    _ => return Ok(()),
                };
                let metadata = serde_json::to_vec(&Metadata {
                    rule: spec.id.index(),
                    kind: v.kind,
                    origin: reference(v.origin),
                    destination: reference(v.destination),
                })?;
                let wkb = codec::encode(&v.geometry)?;
                offsets.write_all(&offset.to_le_bytes())?;
                rows.write_all(&(metadata.len() as u64).to_le_bytes())?;
                rows.write_all(&(wkb.len() as u64).to_le_bytes())?;
                rows.write_all(&metadata)?;
                rows.write_all(&wkb)?;
                offset = offset
                    .checked_add(16 + metadata.len() as u64 + wkb.len() as u64)
                    .context("report offset overflow")?;
                Ok(())
            })
            .map_err(sink_error)?;
        rows.flush()?;
        offsets.flush()?;
        Ok(Self {
            storage,
            summary,
            layers,
        })
    }
    /// Number of aggregated violations.
    pub fn violation_count(&self) -> usize {
        self.summary.violation_count()
    }
    /// Whether every registered rule passed.
    pub fn is_valid(&self) -> bool {
        self.summary.is_valid()
    }
    /// Read a row by its stable zero-based violation ID.
    ///
    /// # Errors
    /// Returns an out-of-range ID, I/O or decoding error.
    pub fn row(&self, index: usize) -> Result<(usize, Violation)> {
        ensure!(index < self.violation_count(), "unknown violation");
        let mut offsets = File::open(self.storage.path().join("offsets.bin"))?;
        let position = (index as u64)
            .checked_mul(8)
            .context("row index overflow")?;
        offsets.seek(SeekFrom::Start(position))?;
        let mut bytes = [0; 8];
        offsets.read_exact(&mut bytes)?;
        let mut rows = File::open(self.storage.path().join("rows.bin"))?;
        rows.seek(SeekFrom::Start(u64::from_le_bytes(bytes)))?;
        self.read_row(&mut rows)
    }
    fn read_row(&self, input: &mut impl Read) -> Result<(usize, Violation)> {
        read_row(input, &self.layers, |index| {
            Ok(self
                .summary
                .rules
                .get(index)
                .context("invalid stored rule")?
                .spec
                .rule)
        })
    }
    /// Iterate rows sequentially without rereading or retaining the disk index.
    pub fn rows(&self) -> impl Iterator<Item = Result<(usize, Violation)>> + '_ {
        let input = File::open(self.storage.path().join("rows.bin"));
        let mut input = input.map(BufReader::new);
        (0..self.violation_count()).map(move |_| match &mut input {
            Ok(input) => self.read_row(input),
            Err(e) => Err(anyhow::anyhow!("opening report: {e}")),
        })
    }
}

/// The stored rows of one finished rule, read back from disk in order.
#[derive(Debug)]
pub struct RuleRows<'a> {
    writer: &'a mut BufWriter<File>,
    path: &'a Path,
    start: u64,
    summary: &'a RuleSummary,
    layers: &'a [LayerId],
    left: usize,
    input: Option<BufReader<File>>,
}
impl RuleRows<'_> {
    fn read(&mut self) -> Result<Violation> {
        let input = match &mut self.input {
            Some(input) => input,
            None => {
                self.writer.flush()?;
                let mut file = File::open(self.path).context("opening report")?;
                file.seek(SeekFrom::Start(self.start))?;
                self.input.insert(BufReader::new(file))
            }
        };
        let spec = self.summary.spec;
        let (_, v) = read_row(input, self.layers, |index| {
            ensure!(index == spec.id.index(), "stored rule order");
            Ok(spec.rule)
        })?;
        Ok(v)
    }
}
impl Iterator for RuleRows<'_> {
    type Item = Result<Violation>;
    fn next(&mut self) -> Option<Self::Item> {
        self.left = self.left.checked_sub(1)?;
        let row = self.read();
        if row.is_err() {
            self.left = 0;
        }
        Some(row)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}
fn read_row(
    input: &mut impl Read,
    layers: &[LayerId],
    rule: impl FnOnce(usize) -> Result<Rule>,
) -> Result<(usize, Violation)> {
    let mut bytes = [0; 8];
    input.read_exact(&mut bytes)?;
    let metadata_length = usize::try_from(u64::from_le_bytes(bytes))?;
    input.read_exact(&mut bytes)?;
    let wkb_length = usize::try_from(u64::from_le_bytes(bytes))?;
    let mut metadata = vec![0; metadata_length];
    input.read_exact(&mut metadata)?;
    let metadata: Metadata = serde_json::from_slice(&metadata)?;
    let mut wkb = vec![0; wkb_length];
    input.read_exact(&mut wkb)?;
    let decode = |r: Option<(usize, i64, Option<u32>)>| -> Result<Option<FeatureRef>> {
        r.map(|(layer, fid, part)| {
            Ok(FeatureRef::new(
                *layers.get(layer).context("invalid stored layer")?,
                fid,
                part,
            ))
        })
        .transpose()
    };
    Ok((
        metadata.rule,
        Violation::new(
            rule(metadata.rule)?,
            metadata.kind,
            codec::decode(&wkb)?.geometry,
            decode(metadata.origin)?,
            decode(metadata.destination)?,
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use topology_checker_core::{Error, SinkError};

    #[test]
    fn sink_errors_keep_their_original_message() {
        let consumer = sink_error(SinkError::Consumer(anyhow::anyhow!("disk full")));
        assert_eq!(consumer.to_string(), "disk full");
        let core = sink_error(SinkError::Core(Error::Cancelled));
        assert_eq!(core.to_string(), "validation cancelled");
    }
}

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
    FeatureRef, LayerId, ReportSummary, Topology, Violation, ViolationKind,
};

#[derive(Debug, Serialize, Deserialize)]
struct Metadata {
    rule: usize,
    kind: ViolationKind,
    origin: Option<(usize, i64, Option<u32>)>,
    destination: Option<(usize, i64, Option<u32>)>,
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
        let mut builder = tempfile::Builder::new();
        builder.prefix("topology-report-");
        let storage = match parent {
            Some(root) => builder.tempdir_in(root)?,
            None => builder.tempdir()?,
        };
        let mut rows = BufWriter::new(File::create(storage.path().join("rows.bin"))?);
        let mut offsets = BufWriter::new(File::create(storage.path().join("offsets.bin"))?);
        let mut offset = 0_u64;
        let summary = topology
            .validate_into(|spec, v| -> Result<()> {
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
            .map_err(|e| match e {
                topology_checker_core::SinkError::Core(e) => anyhow::Error::new(e),
                topology_checker_core::SinkError::Consumer(e) => e,
            })?;
        rows.flush()?;
        offsets.flush()?;
        Ok(Self {
            storage,
            summary,
            layers: topology.layers().map(|(id, _)| id).collect(),
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
        offsets.seek(SeekFrom::Start(
            (index as u64)
                .checked_mul(8)
                .context("row index overflow")?,
        ))?;
        let mut bytes = [0; 8];
        offsets.read_exact(&mut bytes)?;
        let mut rows = File::open(self.storage.path().join("rows.bin"))?;
        rows.seek(SeekFrom::Start(u64::from_le_bytes(bytes)))?;
        self.read_row(&mut rows)
    }
    fn read_row(&self, input: &mut impl Read) -> Result<(usize, Violation)> {
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
                    *self.layers.get(layer).context("invalid stored layer")?,
                    fid,
                    part,
                ))
            })
            .transpose()
        };
        let rule = self
            .summary
            .rules
            .get(metadata.rule)
            .context("invalid stored rule")?
            .spec
            .rule;
        Ok((
            metadata.rule,
            Violation::new(
                rule,
                metadata.kind,
                codec::decode(&wkb)?.geometry,
                decode(metadata.origin)?,
                decode(metadata.destination)?,
            ),
        ))
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

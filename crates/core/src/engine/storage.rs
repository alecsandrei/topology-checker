//! Borrow unchanged cleaned parts from shared immutable original geometry storage.
use geo::{Geometry, LineString, Polygon};
use std::{ops::Deref, sync::Arc};

pub(crate) trait View: Clone {
    fn view<'a>(geometry: &'a Geometry, path: &[usize]) -> &'a Self;
}
impl View for LineString {
    fn view<'a>(mut geometry: &'a Geometry, mut path: &[usize]) -> &'a Self {
        loop {
            match geometry {
                Geometry::LineString(line) => return line,
                Geometry::MultiLineString(lines) => return &lines.0[path[0]],
                Geometry::Polygon(p) => {
                    return if path[0] == 0 {
                        p.exterior()
                    } else {
                        &p.interiors()[path[0] - 1]
                    };
                }
                Geometry::MultiPolygon(ps) => {
                    let p = &ps.0[path[0]];
                    return if path[1] == 0 {
                        p.exterior()
                    } else {
                        &p.interiors()[path[1] - 1]
                    };
                }
                Geometry::GeometryCollection(collection) => {
                    geometry = &collection.0[path[0]];
                    path = &path[1..];
                }
                _ => unreachable!("line source path"),
            }
        }
    }
}
impl View for Polygon {
    fn view<'a>(mut geometry: &'a Geometry, mut path: &[usize]) -> &'a Self {
        loop {
            match geometry {
                Geometry::Polygon(polygon) => return polygon,
                Geometry::MultiPolygon(polygons) => return &polygons.0[path[0]],
                Geometry::GeometryCollection(collection) => {
                    geometry = &collection.0[path[0]];
                    path = &path[1..];
                }
                _ => unreachable!("polygon source path"),
            }
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Shared<G> {
    Source {
        geometries: Arc<Vec<Geometry>>,
        feature: usize,
        path: Vec<usize>,
    },
    Cleaned(G),
}
impl<G: View> Deref for Shared<G> {
    type Target = G;
    fn deref(&self) -> &G {
        match self {
            Self::Source {
                geometries,
                feature,
                path,
            } => G::view(&geometries[*feature], path),
            Self::Cleaned(g) => g,
        }
    }
}
impl<G: View> AsRef<G> for Shared<G> {
    fn as_ref(&self) -> &G {
        self
    }
}

impl Shared<Polygon> {
    pub fn boundary(&self, ring: usize) -> Shared<LineString> {
        match self {
            Self::Source {
                geometries,
                feature,
                path,
            } => {
                let mut path = path.clone();
                path.push(ring);
                Shared::Source {
                    geometries: Arc::clone(geometries),
                    feature: *feature,
                    path,
                }
            }
            Self::Cleaned(p) => Shared::Cleaned(if ring == 0 {
                p.exterior().clone()
            } else {
                p.interiors()[ring - 1].clone()
            }),
        }
    }
}

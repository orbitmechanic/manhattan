//! Camera collision against the building height grid.
//!
//! `scripts/build_collision_grid.py` bakes one height value per 4 m cell from
//! the block art master.  A column is solid from the ground up to that height,
//! which is all a fly-through camera needs: it can fly over roofs, but it can
//! never pass through a wall.  Movement is swept and the remaining motion is
//! projected onto the wall tangent, so the camera slides along a facade instead
//! of stopping dead or penetrating it.

use std::sync::OnceLock;

use bevy::prelude::*;

/// Embedded so the viewer works from any working directory.
const GRID_BYTES: &[u8] = include_bytes!("../assets/models/collision_grid.bin");
const GRID_MAGIC: &[u8; 4] = b"MCG1";

/// The camera is treated as a sphere of this radius when resolving walls.
pub const CAMERA_RADIUS: f32 = 2.4;
/// Never let the camera settle closer than this to a roof it lands on.
const ROOF_CLEARANCE: f32 = 1.2;
const DIAG: f32 = std::f32::consts::FRAC_1_SQRT_2;

#[derive(Resource)]
pub struct CollisionGrid {
    width: usize,
    height: usize,
    min_x: f32,
    min_z: f32,
    cell: f32,
    heights: Vec<u16>,
    max_height: f32,
}

static GRID: OnceLock<CollisionGrid> = OnceLock::new();

impl Default for CollisionGrid {
    fn default() -> Self {
        GRID.get_or_init(|| {
            CollisionGrid::from_bytes(GRID_BYTES).unwrap_or_else(|| {
                warn!("collision grid missing or invalid; camera collision disabled");
                CollisionGrid::empty()
            })
        })
        .clone()
    }
}

impl Clone for CollisionGrid {
    fn clone(&self) -> Self {
        Self {
            width: self.width,
            height: self.height,
            min_x: self.min_x,
            min_z: self.min_z,
            cell: self.cell,
            heights: self.heights.clone(),
            max_height: self.max_height,
        }
    }
}

impl CollisionGrid {
    fn empty() -> Self {
        Self {
            width: 0,
            height: 0,
            min_x: 0.0,
            min_z: 0.0,
            cell: 4.0,
            heights: Vec::new(),
            max_height: 0.0,
        }
    }

    /// Parse the little-endian `MCG1` file produced by the Blender script.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 24 || &bytes[0..4] != GRID_MAGIC {
            return None;
        }
        let width = u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize;
        let height = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
        let min_x = f32::from_le_bytes(bytes[12..16].try_into().ok()?);
        let min_z = f32::from_le_bytes(bytes[16..20].try_into().ok()?);
        let cell = f32::from_le_bytes(bytes[20..24].try_into().ok()?);
        let count = width.checked_mul(height)?;
        if cell <= 0.0 || bytes.len() < 24 + count * 2 {
            return None;
        }
        let mut heights = Vec::with_capacity(count);
        for i in 0..count {
            let o = 24 + i * 2;
            heights.push(u16::from_le_bytes([bytes[o], bytes[o + 1]]));
        }
        let max_height = heights.iter().copied().max().unwrap_or(0) as f32 * 0.1;
        Some(Self {
            width,
            height,
            min_x,
            min_z,
            cell,
            heights,
            max_height,
        })
    }

    #[cfg(test)]
    fn from_heights(width: usize, height: usize, cell: f32, heights: Vec<u16>) -> Self {
        let max_height = heights.iter().copied().max().unwrap_or(0) as f32 * 0.1;
        Self {
            width,
            height,
            min_x: 0.0,
            min_z: 0.0,
            cell,
            heights,
            max_height,
        }
    }

    /// Building-top height at a world position (0 outside the grid).
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        if self.width == 0 || self.height == 0 {
            return 0.0;
        }
        let gx = ((x - self.min_x) / self.cell).floor();
        let gz = ((z - self.min_z) / self.cell).floor();
        if gx < 0.0 || gz < 0.0 {
            return 0.0;
        }
        let gx = gx as usize;
        let gz = gz as usize;
        if gx >= self.width || gz >= self.height {
            return 0.0;
        }
        self.heights[gz * self.width + gx] as f32 * 0.1
    }

    /// Find a solid column at least `min_height` metres tall.  Used by the
    /// runtime self-test to prove the real baked grid is loadable.
    pub fn find_solid(&self, min_height: f32) -> Option<Vec3> {
        let threshold = (min_height * 10.0) as u16;
        for gz in (0..self.height).step_by(3) {
            for gx in (0..self.width).step_by(3) {
                if self.heights[gz * self.width + gx] >= threshold {
                    return Some(Vec3::new(
                        self.min_x + (gx as f32 + 0.5) * self.cell,
                        10.0,
                        self.min_z + (gz as f32 + 0.5) * self.cell,
                    ));
                }
            }
        }
        None
    }

    /// Is a camera sphere at `p` overlapping a building column?
    pub fn blocked(&self, p: Vec3, radius: f32) -> bool {
        if self.width == 0 || p.y >= self.max_height + 0.05 {
            return false;
        }
        // Centre plus an 8-point ring keeps the check cheap but conservative.
        const RING: [(f32, f32); 8] = [
            (1.0, 0.0),
            (DIAG, DIAG),
            (0.0, 1.0),
            (-DIAG, DIAG),
            (-1.0, 0.0),
            (-DIAG, -DIAG),
            (0.0, -1.0),
            (DIAG, -DIAG),
        ];
        let eye = p.y.max(0.5);
        if self.height_at(p.x, p.z) > eye {
            return true;
        }
        RING.iter()
            .any(|(dx, dz)| self.height_at(p.x + dx * radius, p.z + dz * radius) > eye)
    }

    /// Horizontal normal pointing away from the nearest blocked cells.
    fn normal_at(&self, p: Vec3, radius: f32) -> Vec3 {
        const RING: [(f32, f32); 8] = [
            (1.0, 0.0),
            (DIAG, DIAG),
            (0.0, 1.0),
            (-DIAG, DIAG),
            (-1.0, 0.0),
            (-DIAG, -DIAG),
            (0.0, -1.0),
            (DIAG, -DIAG),
        ];
        let eye = p.y.max(0.5);
        let mut n = Vec2::ZERO;
        for (dx, dz) in RING {
            if self.height_at(p.x + dx * radius, p.z + dz * radius) > eye {
                n -= Vec2::new(dx, dz);
            }
        }
        if n.length_squared() < 1e-6 {
            // Flat interior: fall back to the height gradient.
            let e = radius.max(self.cell);
            let hx = self.height_at(p.x + e, p.z) - self.height_at(p.x - e, p.z);
            let hz = self.height_at(p.x, p.z + e) - self.height_at(p.x, p.z - e);
            n = -Vec2::new(hx, hz);
        }
        let v = Vec3::new(n.x, 0.0, n.y);
        v.normalize_or_zero()
    }

    /// Move the camera to the nearest free position if it is already inside a
    /// building (for example after a scripted camera jump).
    pub fn push_out(&self, p: Vec3, radius: f32) -> Vec3 {
        if !self.blocked(p, radius) {
            return p;
        }
        let mut best: Option<(f32, Vec3)> = None;
        for ring in 1..=12 {
            let r = radius + ring as f32 * self.cell;
            for k in 0..16 {
                let a = k as f32 / 16.0 * std::f32::consts::TAU;
                let q = Vec3::new(p.x + a.cos() * r, p.y, p.z + a.sin() * r);
                if !self.blocked(q, radius) {
                    let d = q.distance_squared(p);
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, q));
                    }
                }
            }
            if best.is_some() {
                break;
            }
        }
        if let Some((_, q)) = best {
            return q;
        }
        // Completely enclosed: climb above the tallest column as a last resort.
        Vec3::new(p.x, self.max_height + ROOF_CLEARANCE, p.z)
    }

    /// Swept movement with wall sliding.  `delta` is the desired frame motion.
    pub fn resolve(&self, mut pos: Vec3, mut delta: Vec3, radius: f32) -> Vec3 {
        if !delta.is_finite() {
            return pos;
        }
        for _ in 0..5 {
            let len = delta.length();
            if len < 1e-3 {
                break;
            }
            let start = pos;
            let target = pos + delta;

            // Landing on a roof: if we were above the top and are descending
            // into it, rest on the roof instead of sliding sideways.
            let roof = self.height_at(target.x, target.z);
            if roof > 0.0 && target.y < roof + ROOF_CLEARANCE && pos.y >= roof {
                pos.x = target.x;
                pos.z = target.z;
                pos.y = roof + ROOF_CLEARANCE;
                break;
            }

            // Sweep the whole path; a fast camera can jump over a wall even
            // when the endpoint is in open air.
            let steps = (len / (self.cell * 0.5)).ceil().clamp(1.0, 512.0) as usize;
            let mut safe_t = 0.0;
            let mut hit_t = None;
            for i in 1..=steps {
                let t = i as f32 / steps as f32;
                if self.blocked(pos + delta * t, radius) {
                    hit_t = Some(t);
                    break;
                }
                safe_t = t;
            }
            let Some(hit_t) = hit_t else {
                pos = target;
                break;
            };
            pos += delta * safe_t;
            let leftover = delta * (1.0 - safe_t).max(0.0);
            let hit_point = start + delta * hit_t;
            let normal = self.normal_at(hit_point, radius);
            let horizontal = Vec3::new(leftover.x, 0.0, leftover.z);
            let slide = if normal.length_squared() > 1e-6 {
                horizontal - normal * horizontal.dot(normal)
            } else {
                Vec3::ZERO
            };
            delta = Vec3::new(slide.x, leftover.y, slide.z);
            pos += normal * 0.05;
            if self.blocked(pos, radius) {
                pos = self.push_out(pos, radius);
                break;
            }
        }
        // Guarantee the post-condition: never finish inside a building.
        self.push_out(pos, radius)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_grid() -> CollisionGrid {
        // 40x40 cells, 4 m each.  A 12 x 12 cell (48 m) tower at cell 14..26.
        let mut heights = vec![0u16; 40 * 40];
        for z in 14..26 {
            for x in 14..26 {
                heights[z * 40 + x] = 1200; // 120 m
            }
        }
        CollisionGrid::from_heights(40, 40, 4.0, heights)
    }

    #[test]
    fn heights_and_open_space() {
        let g = test_grid();
        assert_eq!(g.height_at(0.0, 0.0), 0.0);
        assert_eq!(g.height_at(60.0, 60.0), 120.0);
        assert!(g.blocked(Vec3::new(60.0, 10.0, 60.0), CAMERA_RADIUS));
        assert!(!g.blocked(Vec3::new(60.0, 130.0, 60.0), CAMERA_RADIUS));
        assert!(!g.blocked(Vec3::new(5.0, 10.0, 5.0), CAMERA_RADIUS));
    }

    #[test]
    fn camera_never_ends_inside_a_wall() {
        let g = test_grid();
        // Start west of the tower, fly straight east into its wall.
        let start = Vec3::new(20.0, 10.0, 60.0);
        let end = g.resolve(start, Vec3::new(400.0, 0.0, 0.0), CAMERA_RADIUS);
        assert!(
            !g.blocked(end, CAMERA_RADIUS),
            "ended inside at {end:?}, height {}",
            g.height_at(end.x, end.z)
        );
        assert!(
            end.x < 56.0 + CAMERA_RADIUS,
            "walked into the tower: {end:?}"
        );
    }

    #[test]
    fn tangential_motion_slides_along_the_wall() {
        let g = test_grid();
        // Push diagonally into the west wall; the north/south component should
        // survive while the eastward component is removed.
        let start = Vec3::new(30.0, 10.0, 60.0);
        let end = g.resolve(start, Vec3::new(300.0, 0.0, 90.0), CAMERA_RADIUS);
        assert!(!g.blocked(end, CAMERA_RADIUS), "inside at {end:?}");
        assert!(end.z > start.z + 10.0, "did not slide along z: {end:?}");
    }

    #[test]
    fn descending_onto_a_roof_rests_on_top() {
        let g = test_grid();
        let start = Vec3::new(60.0, 200.0, 60.0);
        let end = g.resolve(start, Vec3::new(0.0, -100.0, 0.0), CAMERA_RADIUS);
        assert!(!g.blocked(end, CAMERA_RADIUS), "inside roof at {end:?}");
        assert!(
            (end.y - 120.0).abs() < 1.5,
            "expected roof rest, got {end:?}"
        );
    }

    #[test]
    fn push_out_ejects_a_camera_that_starts_inside() {
        let g = test_grid();
        let q = g.push_out(Vec3::new(60.0, 10.0, 60.0), CAMERA_RADIUS);
        assert!(!g.blocked(q, CAMERA_RADIUS), "still inside at {q:?}");
    }
}

#[cfg(test)]
mod real_grid_tests {
    use super::*;

    #[test]
    fn default_grid_outside_city_is_free() {
        let g = CollisionGrid::default();
        let p = Vec3::new(3600.0, -50.0, 4700.0);
        assert_eq!(g.height_at(3600.0, 4700.0), 0.0);
        assert!(!g.blocked(p, CAMERA_RADIUS));
        let q = g.resolve(p, Vec3::ZERO, CAMERA_RADIUS);
        assert_eq!(q, p);
    }
}

//! The DirectXMath/SimpleMath operations `VRKneeboard.cpp` and `RayIntersectsRect.cpp` use,
//! hand-ported in `f32`. Each follows the `_XM_SSE_INTRINSICS_` path (what MSVC builds for
//! both x64 and x86) and keeps its operation order; multiplications by +-1 are written as
//! negations, which IEEE 754 makes exact.

/// SimpleMath `Vector2`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

/// SimpleMath `Vector3`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// SimpleMath `Quaternion`; `Default` is the identity, as its C++ default constructor.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

/// C++ `VRKneeboard::Pose`; `Default` is C++ `Pose {}`: origin, identity orientation.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    pub position: Vec3,
    pub orientation: Quat,
}

impl Vec2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl Vec3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub(crate) const FORWARD: Self = Self::new(0.0, 0.0, -1.0);
    pub(crate) const BACKWARD: Self = Self::new(0.0, 0.0, 1.0);
    pub(crate) const UNIT_X: Self = Self::new(1.0, 0.0, 0.0);
    pub(crate) const UNIT_Y: Self = Self::new(0.0, 1.0, 0.0);

    /// `XMVector3Dot`: `(x + y) + z`.
    pub(crate) fn dot(self, o: Self) -> f32 {
        (self.x * o.x + self.y * o.y) + self.z * o.z
    }

    pub(crate) fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }

    pub(crate) fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }

    pub(crate) fn scale(self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }

    /// SimpleMath `Vector3::Transform(v, q)`, i.e. `XMVector3Rotate`.
    pub(crate) fn rotate(self, q: Quat) -> Self {
        let v = Quat {
            x: self.x,
            y: self.y,
            z: self.z,
            w: 0.0,
        };
        let r = q.conjugate().multiply(v).multiply(q);
        Self::new(r.x, r.y, r.z)
    }
}

impl Default for Quat {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Quat {
    pub(crate) const IDENTITY: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    /// `XMQuaternionConjugate`.
    fn conjugate(self) -> Self {
        Self {
            x: -self.x,
            y: -self.y,
            z: -self.z,
            w: self.w,
        }
    }

    /// `XMQuaternionMultiply(self, b)`, which is the product `b * self`.
    fn multiply(self, b: Self) -> Self {
        let a = self;
        Self {
            x: (b.w * a.x + b.x * a.w) + (b.y * a.z - b.z * a.y),
            y: (b.w * a.y - b.x * a.z) + (b.y * a.w + b.z * a.x),
            z: (b.w * a.z + b.x * a.y) + (b.z * a.w - b.y * a.x),
            w: (b.w * a.w - b.x * a.x) + (-(b.y * a.y) - b.z * a.z),
        }
    }

    /// SimpleMath `Quaternion::CreateFromRotationMatrix`, i.e. `XMQuaternionRotationMatrix`:
    /// picks the largest-magnitude row of the tensor-product matrix and normalizes it.
    pub(crate) fn from_rotation_matrix(m: &Matrix) -> Self {
        let [r0, r1, r2, _] = m.0;
        let (r00, r11, r22) = (r0[0], r1[1], r2[2]);

        // 4*x^2, 4*y^2, 4*z^2, 4*w^2
        let x2 = -r11 + (-r22 + (r00 + 1.0));
        let y2 = r11 + (-r22 + (-r00 + 1.0));
        let z2 = -r11 + (r22 + (-r00 + 1.0));
        let w2 = r11 + (r22 + (r00 + 1.0));
        // 4*x*y, 4*x*z, 4*y*z
        let xy = r0[1] + r1[0];
        let xz = r0[2] + r2[0];
        let yz = r1[2] + r2[1];
        // 4*x*w, 4*y*w, 4*z*w
        let xw = r1[2] - r2[1];
        let yw = r2[0] - r0[2];
        let zw = r0[1] - r1[0];

        let row = if r22 <= 0.0 {
            if r11 - r00 <= 0.0 {
                [x2, xy, xz, xw]
            } else {
                [xy, y2, yz, yw]
            }
        } else if r11 + r00 <= 0.0 {
            [xz, yz, z2, zw]
        } else {
            [xw, yw, zw, w2]
        };

        // XMVector4Length: (x^2 + z^2) + (y^2 + w^2)
        let sq = row.map(|c| c * c);
        let length = ((sq[0] + sq[2]) + (sq[1] + sq[3])).sqrt();
        Self {
            x: row[0] / length,
            y: row[1] / length,
            z: row[2] / length,
            w: row[3] / length,
        }
    }

    /// SimpleMath `Quaternion::ToEuler`: (pitch, yaw, roll).
    pub(crate) fn to_euler(self) -> Vec3 {
        let Self { x, y, z, w } = self;
        let xx = x * x;
        let yy = y * y;
        let zz = z * z;

        let m31 = 2.0 * x * z + 2.0 * y * w;
        let m32 = 2.0 * y * z - 2.0 * x * w;
        let m33 = 1.0 - 2.0 * xx - 2.0 * yy;

        let cy = (m33 * m33 + m31 * m31).sqrt();
        let cx = (-m32).atan2(cy);
        if cy > 16.0 * f32::EPSILON {
            let m12 = 2.0 * x * y + 2.0 * z * w;
            let m22 = 1.0 - 2.0 * xx - 2.0 * zz;

            Vec3::new(cx, m31.atan2(m33), m12.atan2(m22))
        } else {
            let m11 = 1.0 - 2.0 * yy - 2.0 * zz;
            let m21 = 2.0 * x * y - 2.0 * z * w;

            Vec3::new(cx, 0.0, (-m21).atan2(m11))
        }
    }

    /// SimpleMath `Quaternion::CreateFromYawPitchRoll`, i.e.
    /// `XMQuaternionRotationRollPitchYawFromVector`. Test-only: the C++ tests build poses with
    /// it, and one of their quirks depends on its exact rounding.
    #[cfg(test)]
    pub(crate) fn from_yaw_pitch_roll(yaw: f32, pitch: f32, roll: f32) -> Self {
        let (sp, cp) = vector_sin_cos(pitch * 0.5);
        let (sy, cy) = vector_sin_cos(yaw * 0.5);
        let (sr, cr) = vector_sin_cos(roll * 0.5);
        // Q = (P1 * Sign * Y1) * R1 + (P0 * Y0) * R0
        Self {
            x: (cp * sy) * sr + (sp * cy) * cr,
            y: (-sp * cy) * sr + (cp * sy) * cr,
            z: (-sp * sy) * cr + (cp * cy) * sr,
            w: (sp * sy) * sr + (cp * cy) * cr,
        }
    }
}

/// One lane of `XMVectorSinCos`.
#[cfg(test)]
fn vector_sin_cos(v: f32) -> (f32, f32) {
    // XMVectorModAngles; XMVectorRound rounds half to even.
    let x = v - (v * ONE_DIV_TWO_PI).round_ties_even() * TWO_PI;
    // Map to [-pi/2, pi/2] with sin(y) = sin(x), cos(y) = sign * cos(x).
    let (x, sign) = if x.abs() <= HALF_PI {
        (x, 1.0)
    } else {
        (PI.copysign(x) - x, -1.0)
    };
    let (sin, cos) = sin_cos_polynomials(x);
    (sin, cos * sign)
}

// The DirectXMath `XM_PI`, `XM_2PI`, `XM_1DIV2PI` and `XM_PIDIV2` literals round to these
// (`tests::constants_match_directxmath`).
const PI: f32 = std::f32::consts::PI;
const TWO_PI: f32 = std::f32::consts::TAU;
const ONE_DIV_TWO_PI: f32 = std::f32::consts::FRAC_1_PI * 0.5;
const HALF_PI: f32 = std::f32::consts::FRAC_PI_2;

/// The 11-degree minimax sine and 10-degree cosine shared by `XMScalarSinCos` and
/// `XMVectorSinCos`, for `y` in [-pi/2, pi/2]; the cosine is unsigned.
fn sin_cos_polynomials(y: f32) -> (f32, f32) {
    let y2 = y * y;
    let sin = (((((-2.388_985_9e-8 * y2 + 2.752_556_2e-6) * y2 - 0.000_198_408_74) * y2
        + 0.008_333_331)
        * y2
        - 0.166_666_67)
        * y2
        + 1.0)
        * y;
    let cos = ((((-2.605_161_5e-7 * y2 + 2.476_049_5e-5) * y2 - 0.001_388_837_8) * y2
        + 0.041_666_638)
        * y2
        - 0.5)
        * y2
        + 1.0;
    (sin, cos)
}

/// `XMScalarSinCos`.
fn scalar_sin_cos(value: f32) -> (f32, f32) {
    // Map value to y in [-pi, pi], x = 2*pi*quotient + remainder. C++ `static_cast<int>`
    // truncates toward zero, as `as i32` does.
    let quotient = ONE_DIV_TWO_PI * value;
    let quotient = if value >= 0.0 {
        ((quotient + 0.5) as i32) as f32
    } else {
        ((quotient - 0.5) as i32) as f32
    };
    let y = value - TWO_PI * quotient;

    // Map y to [-pi/2, pi/2] with sin(y) = sin(value).
    let (y, sign) = if y > HALF_PI {
        (PI - y, -1.0)
    } else if y < -HALF_PI {
        (-PI - y, -1.0)
    } else {
        (y, 1.0)
    };

    let (sin, cos) = sin_cos_polynomials(y);
    (sin, sign * cos)
}

/// SimpleMath `Matrix`: row-major, row-vector convention (`v * M`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Matrix(pub(crate) [[f32; 4]; 4]);

impl Matrix {
    pub(crate) const IDENTITY: Self = Self([
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]);

    /// `XMMatrixRotationX`.
    pub(crate) fn rotation_x(radians: f32) -> Self {
        let (s, c) = scalar_sin_cos(radians);
        Self([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, c, s, 0.0],
            [0.0, -s, c, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    /// `XMMatrixRotationY`.
    pub(crate) fn rotation_y(radians: f32) -> Self {
        let (s, c) = scalar_sin_cos(radians);
        Self([
            [c, 0.0, -s, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [s, 0.0, c, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    /// `XMMatrixRotationZ`.
    pub(crate) fn rotation_z(radians: f32) -> Self {
        let (s, c) = scalar_sin_cos(radians);
        Self([
            [c, s, 0.0, 0.0],
            [-s, c, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    /// `XMMatrixTranslation`.
    pub(crate) fn translation(v: Vec3) -> Self {
        let mut m = Self::IDENTITY;
        m.0[3] = [v.x, v.y, v.z, 1.0];
        m
    }

    /// SimpleMath `Matrix::Translation()`.
    pub(crate) fn get_translation(&self) -> Vec3 {
        let [x, y, z, _] = self.0[3];
        Vec3::new(x, y, z)
    }

    /// `XMMatrixMultiply`: each element is `(x*b0 + z*b2) + (y*b1 + w*b3)`.
    pub(crate) fn mul(&self, b: &Self) -> Self {
        let b = &b.0;
        Self(self.0.map(|[x, y, z, w]| {
            std::array::from_fn(|j| (x * b[0][j] + z * b[2][j]) + (y * b[1][j] + w * b[3][j]))
        }))
    }
}

/// SimpleMath `Plane(point, normal)`, i.e. `XMPlaneFromPointNormal`: `(n, -dot(p, n))`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Plane {
    normal: Vec3,
    d: f32,
}

impl Plane {
    pub(crate) fn from_point_normal(point: Vec3, normal: Vec3) -> Self {
        Self {
            normal,
            d: -point.dot(normal),
        }
    }
}

/// SimpleMath `Ray::Intersects(Plane, float&)`: the distance along `direction`, if any.
pub(crate) fn ray_intersects_plane(position: Vec3, direction: Vec3, plane: Plane) -> Option<f32> {
    // DirectXCollision.inl `g_RayEpsilon`
    const RAY_EPSILON: f32 = 1e-20;

    let nd = plane.normal.dot(direction);
    if nd.abs() <= RAY_EPSILON {
        return None;
    }
    // t = -(dot(n, origin) + D) / dot(n, dir)
    let dist = -((plane.normal.dot(position) + plane.d) / nd);
    // NaN is not < 0, so it counts as a hit, as in C++.
    if dist < 0.0 { None } else { Some(dist) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::excessive_precision, clippy::approx_constant)]
    fn constants_match_directxmath() {
        assert_eq!(PI.to_bits(), 3.141_592_654_f32.to_bits());
        assert_eq!(TWO_PI.to_bits(), 6.283_185_307_f32.to_bits());
        assert_eq!(ONE_DIV_TWO_PI.to_bits(), 0.159_154_943_f32.to_bits());
        assert_eq!(HALF_PI.to_bits(), 1.570_796_327_f32.to_bits());
    }
}

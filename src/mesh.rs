use super::*;
use geo::algorithm::contains::Contains;
use geo::{LineString, Point, Polygon};

#[derive(Deserialize)]
pub struct Mesh2DInput {
    pub length_unit: String,
    pub coordinate_sets: Vec<(f64, f64, f64, f64, f64, f64)>,
    pub boundary_points: Vec<(f64, f64)>,
    pub simulation_boundary_points: Vec<(f64, f64)>,
    pub densities: Vec<Vec<f64>>,
}

pub struct Mesh2D {
    mesh: Vec<Cell2D>,
    pub boundary: Polygon<f64>,
    pub simulation_boundary: Polygon<f64>,
}
impl Mesh2D {
    pub fn new(mesh_2d_input: Mesh2DInput) -> Mesh2D {
        let coordinate_sets = mesh_2d_input.coordinate_sets;
        let boundary_points = mesh_2d_input.boundary_points;
        let simulation_boundary_points = mesh_2d_input.simulation_boundary_points;
        let densities = mesh_2d_input.densities;

        assert_eq!(
            coordinate_sets.len(),
            densities.len(),
            "Input error: coordinates and data of unequal length."
        );
        let n = coordinate_sets.len();

        let mut cells: Vec<Cell2D> = Vec::with_capacity(n);

        //Multiply all coordinates by value of geometry unit.
        let length_unit: f64 = match mesh_2d_input.length_unit.as_str() {
            "MICRON" => MICRON,
            "CM" => CM,
            "ANGSTROM" => ANGSTROM,
            "NM" => NM,
            "M" => 1.,
            _ => panic!("Input error: incorrect unit {} in input file. Choose one of: MICRON, CM, ANGSTROM, NM, M",
                mesh_2d_input.length_unit.as_str())
        };

        for (coordinate_set, densities) in coordinate_sets.iter().zip(densities) {
            let coordinate_set_converted = (
                coordinate_set.0 * length_unit,
                coordinate_set.1 * length_unit,
                coordinate_set.2 * length_unit,
                coordinate_set.3 * length_unit,
                coordinate_set.4 * length_unit,
                coordinate_set.5 * length_unit,
            );

            let total_density: f64 = densities.iter().sum();
            let concentrations: Vec<f64> = densities
                .iter()
                .map(|&i| i / total_density)
                .collect::<Vec<f64>>();

            cells.push(Cell2D::new(
                coordinate_set_converted,
                densities,
                concentrations,
            ));
        }

        let mut boundary_points_converted = Vec::with_capacity(boundary_points.len());
        for (x, y) in boundary_points.iter() {
            boundary_points_converted.push((x * length_unit, y * length_unit));
        }
        let boundary: Polygon<f64> =
            Polygon::new(LineString::from(boundary_points_converted), vec![]);

        let mut simulation_boundary_points_converted =
            Vec::with_capacity(simulation_boundary_points.len());
        for (x, y) in simulation_boundary_points.iter() {
            simulation_boundary_points_converted.push((x * length_unit, y * length_unit));
        }
        let simulation_boundary: Polygon<f64> = Polygon::new(
            LineString::from(simulation_boundary_points_converted),
            vec![],
        );

        Mesh2D {
            mesh: cells,
            boundary: boundary,
            simulation_boundary: simulation_boundary,
        }
    }

    pub fn get_densities(&self, x: f64, y: f64) -> &Vec<f64> {
        for cell in &self.mesh {
            if cell.contains(x, y) {
                return &cell.densities;
            }
        }
        panic!(
            "Geometry error: point ({}, {}) not found in any cell of the mesh.",
            x, y
        );
    }

    pub fn get_total_density(&self, x: f64, y: f64) -> f64 {
        for cell in &self.mesh {
            if cell.contains(x, y) {
                return cell.densities.iter().sum::<f64>();
            }
        }
        panic!(
            "Geometry error: point ({}, {}) not found in any cell of the mesh.",
            x, y
        );
    }

    pub fn get_concentrations(&self, x: f64, y: f64) -> &Vec<f64> {
        if self.inside(x, y) {
            for cell in &self.mesh {
                if cell.contains(x, y) {
                    return &cell.concentrations;
                }
            }
            panic!("Geometry error: method inside() is returning true for points outside all cells. Check boundary points.")
        } else {
            return &self.nearest_cell_to(x, y).concentrations;
        }
    }

    pub fn inside(&self, x: f64, y: f64) -> bool {
        self.boundary.contains(&Point::new(x, y))
    }

    pub fn nearest_cell_to(&self, x: f64, y: f64) -> &Cell2D {
        let mut min_distance: f64 = std::f64::MAX;
        let mut index: usize = 0;

        for (cell_index, cell) in self.mesh.iter().enumerate() {
            let distance_to = cell.distance_to(x, y);
            if distance_to < min_distance {
                min_distance = distance_to;
                index = cell_index;
            }
        }

        return &self.mesh[index];
    }
}

pub struct Cell2D {
    triangle: Triangle2D,
    pub densities: Vec<f64>,
    pub concentrations: Vec<f64>,
}
impl Cell2D {
    pub fn new(
        coordinate_set: (f64, f64, f64, f64, f64, f64),
        densities: Vec<f64>,
        concentrations: Vec<f64>,
    ) -> Cell2D {
        Cell2D {
            triangle: Triangle2D::new(coordinate_set),
            densities: densities,
            concentrations: concentrations,
        }
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        self.triangle.contains(x, y)
    }

    pub fn distance_to(&self, x: f64, y: f64) -> f64 {
        self.triangle.distance_to(x, y)
    }
}

pub struct Triangle2D {
    x1: f64,
    x2: f64,
    x3: f64,
    y1: f64,
    y2: f64,
    y3: f64,
    segments: Vec<(f64, f64, f64, f64)>,
}

impl Triangle2D {
    pub fn new(coordinate_set: (f64, f64, f64, f64, f64, f64)) -> Triangle2D {
        let x1 = coordinate_set.0;
        let x2 = coordinate_set.1;
        let x3 = coordinate_set.2;

        let y1 = coordinate_set.3;
        let y2 = coordinate_set.4;
        let y3 = coordinate_set.5;

        Triangle2D {
            x1,
            x2,
            x3,
            y1,
            y2,
            y3,
            segments: vec![(x1, y1, x2, y2), (x2, y2, x3, y3), (x3, y3, x1, y1)],
        }
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        let x1 = self.x1;
        let x2 = self.x2;
        let x3 = self.x3;
        let y1 = self.y1;
        let y2 = self.y2;
        let y3 = self.y3;

        let a = ((y2 - y3) * (x - x3) + (x3 - x2) * (y - y3))
            / ((y2 - y3) * (x1 - x3) + (x3 - x2) * (y1 - y3));
        let b = ((y3 - y1) * (x - x3) + (x1 - x3) * (y - y3))
            / ((y2 - y3) * (x1 - x3) + (x3 - x2) * (y1 - y3));
        let c = 1. - a - b;

        (0. <= a) & (a <= 1.) & (0. <= b) & (b <= 1.) & (0. <= c) & (c <= 1.)
    }

    pub fn centroid(&self) -> (f64, f64) {
        (
            (self.x1 + self.x2 + self.x3) / 3.,
            (self.y1 + self.y2 + self.y3) / 3.,
        )
    }

    pub fn distance_to(&self, x: f64, y: f64) -> f64 {
        let mut distance_to = std::f64::MAX;

        for segment in &self.segments {
            let length_2 = (segment.2 - segment.0) * (segment.2 - segment.0)
                + (segment.3 - segment.1) * (segment.3 - segment.1);

            assert!(
                length_2 != 0.,
                "Geometry error: mesh contains triangle with zero-length side."
            );

            let u = ((x - segment.0) * (segment.2 - segment.0)
                + (y - segment.1) * (segment.3 - segment.1))
                / length_2;

            let distance = if u < 0. {
                ((x - segment.0) * (x - segment.0) + (y - segment.1) * (y - segment.1)).sqrt()
            } else if u > 1. {
                ((x - segment.2) * (x - segment.2) + (y - segment.3) * (y - segment.3)).sqrt()
            } else {
                let x_intersect = segment.0 + u * (segment.2 - segment.0);
                let y_intersect = segment.1 + u * (segment.3 - segment.1);

                ((x - x_intersect) * (x - x_intersect) + (y - y_intersect) * (y - y_intersect))
                    .sqrt()
            };

            if distance < distance_to {
                distance_to = distance;
            }
        }
        distance_to
    }

    pub fn distance_to_center(&self, x: f64, y: f64) -> f64 {
        let centroid = self.centroid();
        ((x - centroid.0) * (x - centroid.0) + (y - centroid.1) * (y - centroid.1)).sqrt()
    }
}

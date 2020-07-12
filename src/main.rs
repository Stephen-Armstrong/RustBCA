#![allow(unused_variables)]
#![allow(non_snake_case)]

//Error handling crate
//use anyhow::Result;
use anyhow::Result;
use anyhow::*;

//Geometry crate
use geo::algorithm::contains::Contains;
//use geo::algorithm::convexhull::ConvexHull;
use geo::algorithm::closest_point::ClosestPoint;
use geo::{point, Polygon, LineString, Closest, Point};// MultiPoint, Point};

//Serializing/Deserializing crate
use serde::*;

//Array input via hdf5
use hdf5::*;

//Parallelization
use rayon::prelude::*;
use rayon::*;

//I/O
use std::fs::OpenOptions;
use std::io::prelude::*;
use std::io::BufWriter;
use std::f64::consts::PI;

//Load internal modules
pub mod material;
pub mod particle;
pub mod tests;
pub mod interactions;
pub mod bca;
pub mod mesh;
use crate::particle::ParticleInput;

//Physical constants
const Q: f64 = 1.60217646E-19;
const EV: f64 = Q;
const AMU: f64 = 1.660539E-27;
const ANGSTROM: f64 = 1E-10;
const MICRON: f64 = 1E-6;
const NM: f64 = 1E-9;
const CM: f64 = 1E-2;
const EPS0: f64 = 8.85418781E-12;
const A0: f64 = 5.29177211E-11;
//const K: f64 = 1.11265E-10;
const ME: f64 =  9.109383632E-31;
const SQRTPI: f64 = 1.772453850906;
const SQRT2PI: f64 = 2.506628274631;
const C: f64 = 299792000.;
const BETHE_BLOCH_PREFACTOR: f64 = 4.*PI*(Q*Q/(4.*PI*EPS0))*(Q*Q/(4.*PI*EPS0))/ME/C/C;
const LINDHARD_SCHARFF_PREFACTOR: f64 = 1.212*ANGSTROM*ANGSTROM*Q;
const LINDHARD_REDUCED_ENERGY_PREFACTOR: f64 = 4.*PI*EPS0/Q/Q;

//Electronic stopping models
const INTERPOLATED: i32 = 0;
const LOW_ENERGY_NONLOCAL: i32 = 1;
const LOW_ENERGY_LOCAL: i32 = 2;
const LOW_ENERGY_EQUIPARTITION: i32 = 3;

//Mean free path models
const LIQUID: i32 = 0;
const GASEOUS: i32 = 1;

//Interaction potentials
const MOLIERE: i32 = 0;
const KR_C: i32 = 1;
const ZBL: i32 = 2;
const LENZ_JENSEN: i32 = 3;
const TRIDYN: i32 = -1;

//Scattering integral forms
const QUADRATURE: i32 = 0;
const MAGIC: i32 = 1;

#[derive(Clone)]
pub struct Vector {
    x: f64,
    y: f64,
    z: f64,
}
impl Vector {
    pub fn new(x: f64, y: f64, z: f64) -> Vector {
        Vector {
            x: x,
            y: y,
            z: z
        }
    }
    fn magnitude(&self) -> f64 {
        return (self.x*self.x + self.y*self.y + self.z*self.z).sqrt();
    }
    fn assign(&mut self, other: &Vector) {
        self.x = other.x;
        self.y = other.y;
        self.z = other.z;
    }

    fn dot(&self, other: &Vector) -> f64 {
        return self.x*other.x + self.y*other.y + self.z*other.z;
    }

    fn normalize(&mut self) {
        let magnitude = self.magnitude();
        self.x /= magnitude;
        self.y /= magnitude;
        self.z /= magnitude;
    }

    fn add(&self, other: &Vector) -> Vector {
        Vector::new(
            self.x + other.x,
            self.y + other.y,
            self.z + other.z,
        )
    }
}

#[derive(Clone)]
pub struct Vector4 {
    E: f64,
    x: f64,
    y: f64,
    z: f64,
}
impl Vector4 {
    fn new(E: f64, x: f64, y: f64, z: f64) -> Vector4 {
        Vector4 {
            E: E,
            x: x,
            y: y,
            z: z
        }
    }
}

#[derive(Deserialize)]
pub struct Input {
    options: Options,
    material_parameters: material::MaterialParameters,
    particle_parameters: particle::ParticleParameters,
    mesh_2d_input: mesh::Mesh2DInput,
}

#[derive(Deserialize)]
pub struct Options {
    name: String,
    track_trajectories: bool,
    track_recoils: bool,
    track_recoil_trajectories: bool,
    write_files: bool,
    stream_size: usize,
    print: bool,
    print_num: usize,
    weak_collision_order: usize,
    suppress_deep_recoils: bool,
    high_energy_free_flight_paths: bool,
    electronic_stopping_mode: i32,
    mean_free_path_model: i32,
    interaction_potential: i32,
    scattering_integral: i32,
    tolerance: f64,
    max_iterations: usize
}

fn main() {
    //Read input file, convert to string, and open with toml
    let mut input_toml = String::new();
    let mut file = OpenOptions::new()
        .read(true)
        .write(false)
        .create(false)
        .open("input.toml")
        .expect("Input errror: could not open input file, input.toml.");
    file.read_to_string(&mut input_toml).unwrap();
    let input: Input = toml::from_str(&input_toml).unwrap();

    //Unpack toml information into structs
    let material = material::Material::new(input.material_parameters, input.mesh_2d_input);
    assert!(material.n.len() == material.m.len(), "Input error: material input arrays of unequal length.");
    assert!(material.n.len() == material.Z.len(), "Input error: material input arrays of unequal length.");
    assert!(material.n.len() == material.Eb.len(), "Input error: material input arrays of unequal length.");
    assert!(material.n.len() == material.Es.len(), "Input error: material input arrays of unequal length.");

    let options = input.options;
    let particle_parameters = input.particle_parameters;

    //Check that incompatible options are not on simultaneously
    if options.high_energy_free_flight_paths {
        assert!(options.electronic_stopping_mode == INTERPOLATED,
            "Input error: High energy free flight paths used with low energy stoppping power.");
    }

    if options.electronic_stopping_mode == INTERPOLATED {
        assert!(options.weak_collision_order == 0,
            "Input error: Cannot use weak collision loop with free flight paths.");
        //assert!(options.mean_free_path_model == LIQUID,
        //    "Gaseous model not currently implemented for high energy free flight paths.");
    }
    if options.mean_free_path_model == GASEOUS {
        assert!(options.weak_collision_order == 0,
            "Input error: Cannot use weak collisions with gaseous mean free path model.");
    }

    //Check that particle arrays are equal length
    assert_eq!(particle_parameters.Z.len(), particle_parameters.m.len(),
        "Input error: particle input arrays of unequal length.");
    assert_eq!(particle_parameters.Z.len(), particle_parameters.E.len(),
        "Input error: particle input arrays of unequal length.");
    assert_eq!(particle_parameters.Z.len(), particle_parameters.pos.len(),
        "Input error: particle input arrays of unequal length.");
    assert_eq!(particle_parameters.Z.len(), particle_parameters.dir.len(),
        "Input error: particle input arrays of unequal length.");

    let N = particle_parameters.Z.len();

    //Determine the length, energy, and mass units for particle input
    let length_unit: f64 = match particle_parameters.length_unit.as_str() {
        "MICRON" => MICRON,
        "CM" => CM,
        "ANGSTROM" => ANGSTROM,
        "NM" => NM,
        "M" => 1.,
        _ => panic!("Input error: unknown unit {} in input file. Choose one of: MICRON, CM, ANGSTROM, NM, M",
            particle_parameters.length_unit.as_str())
    };
    let energy_unit: f64 = match particle_parameters.energy_unit.as_str() {
        "EV" => EV,
        "J"  => 1.,
        "KEV" => EV*1E3,
        "MEV" => EV*1E6,
        _ => panic!("Input error: unknown unit {} in input file. Choose one of: EV, J, KEV, MEV",
            particle_parameters.energy_unit.as_str())
    };
    let mass_unit: f64 = match particle_parameters.mass_unit.as_str() {
        "AMU" => AMU,
        "KG" => 1.0,
        _ => panic!("Input error: unknown unit {} in input file. Choose one of: AMU, KG",
            particle_parameters.mass_unit.as_str())
    };
    //let particle_input_filename = particle_parameters.particle_input_filename.as_str();

    //Estimate maximum number of recoils produced per ion
    let mut max_energy: f64 = 0.;
    let mut total_particles: usize = 0;
    for particle_index in 0..N {
        let E = particle_parameters.E[particle_index];
        let N_ = particle_parameters.N[particle_index];
        if E > max_energy {
            max_energy = E*energy_unit;
        }
        total_particles += N_;
    }

    //Create particle vector from input file
    let estimated_num_particles: usize = match options.track_recoils {
        true => total_particles + ((max_energy/material.minimum_cutoff_energy()).ceil() as usize),
        false => total_particles,
    };

    //Read in HDF5 (Experimental)
    //let _e = hdf5::silence_errors();
    //let particle_input_file = hdf5::File::open(particle_input_filename).unwrap();
    //let particle_input = particle_input_file.dataset("particles").unwrap();
    //let particle_input_array = particle_input.read_raw::<ParticleInput>().unwrap();

    //let mut particles: Vec<particle::Particle> = Vec::with_capacity(estimated_num_particles)

    let mut particles: Vec<particle::Particle> = Vec::new();
    for particle_index in 0..N {

        let N_ = particle_parameters.N[particle_index];
        let m = particle_parameters.m[particle_index];
        let Z = particle_parameters.Z[particle_index];
        let E = particle_parameters.E[particle_index];
        let Ec = particle_parameters.Ec[particle_index];
        let Es = particle_parameters.Es[particle_index];
        let (x, y, z) = particle_parameters.pos[particle_index];
        let (cosx, cosy, cosz) = particle_parameters.dir[particle_index];
        for sub_particle_index in 0..N_ {
            //Add new particle to particle vector
            particles.push(particle::Particle::new(
                m*mass_unit, Z, E*energy_unit, Ec*energy_unit, Es*energy_unit,
                x*length_unit, y*length_unit, z*length_unit,
                cosx, cosy, cosz, true, options.track_trajectories
            ));
        }
    }

    //for particle in particle_input_array {
    //    particles.push(
    //        particle::Particle::new(
    //            particle.m*mass_unit, particle.Z, particle.E*energy_unit,
    //            particle.Ec*energy_unit, particle.Es*energy_unit,
    //            particle.x*length_unit, particle.y*length_unit, particle.z*length_unit,
    //            particle.ux, particle.uy, particle.uz,
    //            true, options.track_trajectories
    //        )
    //    );
    //}

    //let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build_global().unwrap();

    //Main loop, parallelized with rayon
    let mut finished_particles: Vec<particle::Particle> = Vec::new();
    finished_particles.par_extend(
        particles.into_par_iter().map(
            |particle| bca::single_ion_bca(particle.clone(), &material, &options))
            .flatten()
    );

    //Open output files for streaming output
    let reflected_file = OpenOptions::new()
        .write(true).
        create(true).
        open(format!("{}{}", options.name, "reflected.output")).
        unwrap();
    let mut reflected_file_stream = BufWriter::with_capacity(options.stream_size, reflected_file);

    let sputtered_file = OpenOptions::new()
        .write(true)
        .create(true)
        .open(format!("{}{}", options.name, "sputtered.output"))
        .unwrap();
    let mut sputtered_file_stream = BufWriter::with_capacity(options.stream_size, sputtered_file);

    let deposited_file = OpenOptions::new()
        .write(true)
        .create(true)
        .open(format!("{}{}", options.name, "deposited.output"))
        .unwrap();
    let mut deposited_file_stream = BufWriter::with_capacity(options.stream_size, deposited_file);

    let trajectory_file = OpenOptions::new()
        .write(true)
        .create(true)
        .open(format!("{}{}", options.name, "trajectories.output"))
        .unwrap();
    let mut trajectory_file_stream = BufWriter::with_capacity(options.stream_size, trajectory_file);

    let trajectory_data = OpenOptions::new()
        .write(true)
        .create(true)
        .open(format!("{}{}", options.name, "trajectory_data.output"))
        .unwrap();
    let mut trajectory_data_stream = BufWriter::with_capacity(options.stream_size, trajectory_data);


    for particle in finished_particles {
        if particle.incident & particle.left {
            writeln!(
                reflected_file_stream, "{},{},{},{},{},{},{},{},{},{}",
                particle.m/mass_unit, particle.Z, particle.E/energy_unit,
                particle.pos.x/length_unit, particle.pos.y/length_unit, particle.pos.z/length_unit,
                particle.dir.x, particle.dir.y, particle.dir.z,
                particle.number_collision_events
            ).expect("Output error: could not write to reflected.output.");

        }

        //Incident particle, stopped in material: deposited
        if particle.incident & particle.stopped {
            writeln!(
                deposited_file_stream, "{},{},{},{},{},{}",
                particle.m/mass_unit, particle.Z,
                particle.pos.x/length_unit, particle.pos.y/length_unit, particle.pos.z/length_unit,
                particle.number_collision_events
            ).expect("Output error: could not write to deposited.output.");
        }

        //Not an incident particle, left material: sputtered
        if !particle.incident & particle.left {
            writeln!(
                sputtered_file_stream, "{},{},{},{},{},{},{},{},{},{},{},{},{}",
                particle.m/mass_unit, particle.Z, particle.E/energy_unit,
                particle.pos.x/length_unit, particle.pos.y/length_unit, particle.pos.z/length_unit,
                particle.dir.x, particle.dir.y, particle.dir.z,
                particle.number_collision_events,
                particle.pos_origin.x/length_unit, particle.pos_origin.y/length_unit, particle.pos_origin.z/length_unit
            ).expect("Output error: could not write to sputtered.output.");
        }

        //Trajectory output
        if particle.track_trajectories {
            writeln!(trajectory_data_stream, "{}", particle.trajectory.len())
                .expect("Output error: could not write trajectory length data.");

            for pos in particle.trajectory {
                writeln!(
                    trajectory_file_stream, "{},{},{},{},{},{}",
                    particle.m/mass_unit, particle.Z, pos.E/energy_unit,
                    pos.x/length_unit, pos.y/length_unit, pos.z/length_unit,
                ).expect("Output error: could not write to trajectories.output.");
            }
        }
    }
    //Flush all file streams before dropping to ensure all data is written
    reflected_file_stream.flush().unwrap();
    deposited_file_stream.flush().unwrap();
    sputtered_file_stream.flush().unwrap();
    trajectory_data_stream.flush().unwrap();
    trajectory_file_stream.flush().unwrap();
}

fn main_old() {
    //Read input file, convert to string, and open with toml
    let mut input_toml = String::new();
    let mut file = OpenOptions::new()
        .read(true)
        .write(false)
        .create(false)
        .open("input.toml")
        .expect("Input errror: could not open input file, input.toml.");
    file.read_to_string(&mut input_toml).unwrap();
    let input: Input = toml::from_str(&input_toml).unwrap();

    //Unpack toml information into structs
    let material = material::Material::new(input.material_parameters, input.mesh_2d_input);
    assert!(material.n.len() == material.m.len(), "Input error: material input arrays of unequal length.");
    assert!(material.n.len() == material.Z.len(), "Input error: material input arrays of unequal length.");
    assert!(material.n.len() == material.Eb.len(), "Input error: material input arrays of unequal length.");
    assert!(material.n.len() == material.Es.len(), "Input error: material input arrays of unequal length.");

    let options = input.options;
    let particle_parameters = input.particle_parameters;

    //Check that incompatible options are not on simultaneously
    if options.high_energy_free_flight_paths {
        assert!(options.electronic_stopping_mode == INTERPOLATED,
            "Input error: High energy free flight paths used with low energy stoppping power.");
    }

    if options.electronic_stopping_mode == INTERPOLATED {
        assert!(options.weak_collision_order == 0,
            "Input error: Cannot use weak collision loop with free flight paths.");
        //assert!(options.mean_free_path_model == LIQUID,
        //    "Gaseous model not currently implemented for high energy free flight paths.");
    }
    if options.mean_free_path_model == GASEOUS {
        assert!(options.weak_collision_order == 0,
            "Input error: Cannot use weak collisions with gaseous mean free path model.");
    }

    //Check that particle arrays are equal length
    assert_eq!(particle_parameters.Z.len(), particle_parameters.m.len(),
        "Input error: particle input arrays of unequal length.");
    assert_eq!(particle_parameters.Z.len(), particle_parameters.E.len(),
        "Input error: particle input arrays of unequal length.");
    assert_eq!(particle_parameters.Z.len(), particle_parameters.pos.len(),
        "Input error: particle input arrays of unequal length.");
    assert_eq!(particle_parameters.Z.len(), particle_parameters.dir.len(),
        "Input error: particle input arrays of unequal length.");

    let N = particle_parameters.Z.len();

    //Determine the length, energy, and mass units for particle input
    let length_unit: f64 = match particle_parameters.length_unit.as_str() {
        "MICRON" => MICRON,
        "CM" => CM,
        "ANGSTROM" => ANGSTROM,
        "NM" => NM,
        "M" => 1.,
        _ => panic!("Input error: unknown unit {} in input file. Choose one of: MICRON, CM, ANGSTROM, NM, M",
            particle_parameters.length_unit.as_str())
    };
    let energy_unit: f64 = match particle_parameters.energy_unit.as_str() {
        "EV" => EV,
        "J"  => 1.,
        "KEV" => EV*1E3,
        "MEV" => EV*1E6,
        _ => panic!("Input error: unknown unit {} in input file. Choose one of: EV, J, KEV, MEV",
            particle_parameters.energy_unit.as_str())
    };
    let mass_unit: f64 = match particle_parameters.mass_unit.as_str() {
        "AMU" => AMU,
        "KG" => 1.0,
        _ => panic!("Input error: unknown unit {} in input file. Choose one of: AMU, KG",
            particle_parameters.mass_unit.as_str())
    };
    //let particle_input_filename = particle_parameters.particle_input_filename.as_str();

    //Estimate maximum number of recoils produced per ion
    let mut max_energy: f64 = 0.;
    let mut total_particles: usize = 0;
    for particle_index in 0..N {
        let E = particle_parameters.E[particle_index];
        let N_ = particle_parameters.N[particle_index];
        if E > max_energy {
            max_energy = E*energy_unit;
        }
        total_particles += N_;
    }

    //Create particle vector from input file
    let estimated_num_particles: usize = match options.track_recoils {
        true => total_particles + ((max_energy/material.minimum_cutoff_energy()).ceil() as usize),
        false => total_particles,
    };

    //Read in HDF5 (Experimental)
    //let _e = hdf5::silence_errors();
    //let particle_input_file = hdf5::File::open(particle_input_filename).unwrap();
    //let particle_input = particle_input_file.dataset("particles").unwrap();
    //let particle_input_array = particle_input.read_raw::<ParticleInput>().unwrap();

    //let mut particles: Vec<particle::Particle> = Vec::with_capacity(estimated_num_particles)

    let mut particles: Vec<particle::Particle> = Vec::new();
    for particle_index in 0..N {

        let N_ = particle_parameters.N[particle_index];
        let m = particle_parameters.m[particle_index];
        let Z = particle_parameters.Z[particle_index];
        let E = particle_parameters.E[particle_index];
        let Ec = particle_parameters.Ec[particle_index];
        let Es = particle_parameters.Es[particle_index];
        let (x, y, z) = particle_parameters.pos[particle_index];
        let (cosx, cosy, cosz) = particle_parameters.dir[particle_index];
        for sub_particle_index in 0..N_ {
            //Add new particle to particle vector
            particles.push(particle::Particle::new(
                m*mass_unit, Z, E*energy_unit, Ec*energy_unit, Es*energy_unit,
                x*length_unit, y*length_unit, z*length_unit,
                cosx, cosy, cosz, true, options.track_trajectories
            ));
        }
    }

    //for particle in particle_input_array {
    //    particles.push(
    //        particle::Particle::new(
    //            particle.m*mass_unit, particle.Z, particle.E*energy_unit,
    //            particle.Ec*energy_unit, particle.Es*energy_unit,
    //            particle.x*length_unit, particle.y*length_unit, particle.z*length_unit,
    //            particle.ux, particle.uy, particle.uz,
    //            true, options.track_trajectories
    //        )
    //    );
    //}

    //Open output files for streaming output
    let reflected_file = OpenOptions::new()
        .write(true).
        create(true).
        open(format!("{}{}", options.name, "reflected.output")).
        unwrap();
    let mut reflected_file_stream = BufWriter::with_capacity(options.stream_size, reflected_file);

    let sputtered_file = OpenOptions::new()
        .write(true)
        .create(true)
        .open(format!("{}{}", options.name, "sputtered.output"))
        .unwrap();
    let mut sputtered_file_stream = BufWriter::with_capacity(options.stream_size, sputtered_file);

    let deposited_file = OpenOptions::new()
        .write(true)
        .create(true)
        .open(format!("{}{}", options.name, "deposited.output"))
        .unwrap();
    let mut deposited_file_stream = BufWriter::with_capacity(options.stream_size, deposited_file);

    let trajectory_file = OpenOptions::new()
        .write(true)
        .create(true)
        .open(format!("{}{}", options.name, "trajectories.output"))
        .unwrap();
    let mut trajectory_file_stream = BufWriter::with_capacity(options.stream_size, trajectory_file);

    let trajectory_data = OpenOptions::new()
        .write(true)
        .create(true)
        .open(format!("{}{}", options.name, "trajectory_data.output"))
        .unwrap();
    let mut trajectory_data_stream = BufWriter::with_capacity(options.stream_size, trajectory_data);

    //Main loop
    let mut particle_index: usize = particles.len();

    'particle_loop: while particle_index > 0 {
        //Remove particle from top of vector as particle
        let mut particle = particles.pop().unwrap();

        //Print to stdout
        if options.print & particle.incident & (particle_index % (total_particles / options.print_num) == 0){
            println!("Incident Ion {} of {}", particle_index, total_particles);
        }

        //BCA loop
        'trajectory_loop: while !particle.stopped & !particle.left {

            //Choose impact parameters and azimuthal angles for all collisions, and determine mean free path
            let binary_collision_geometries = bca::determine_mfp_phi_impact_parameter(&mut particle, &material, &options);

            let mut total_energy_loss = 0.;
            let mut total_asymptotic_deflection = 0.;
            let mut distance_of_closest_approach = 0.;
            let mut strong_collision_Z = 0.;
            let mut strong_collision_index: usize = 0;

            'collision_loop: for k in 0..options.weak_collision_order + 1 {

                let (species_index, mut particle_2) = bca::choose_collision_partner(&mut particle, &material,
                    &binary_collision_geometries[k], &options);

                //If recoil location is inside, proceed with binary collision loop
                if material.inside(particle_2.pos.x, particle_2.pos.y) & material.inside_energy_barrier(particle.pos.x, particle.pos.y) {

                    //Determine scattering angle from binary collision
                    let binary_collision_result = bca::calculate_binary_collision(&particle,
                        &particle_2, &binary_collision_geometries[k], &options);

                    //Only use 0th order collision for local electronic stopping
                    if k == 0 {
                        distance_of_closest_approach = binary_collision_result.normalized_distance_of_closest_approach;
                        strong_collision_Z = particle_2.Z;
                        strong_collision_index = species_index;
                    }

                    //Energy transfer to recoil
                    particle_2.E = binary_collision_result.recoil_energy - material.average_bulk_binding_energy(particle_2.pos.x, particle_2.pos.y);

                    //Accumulate asymptotic deflections for primary particle
                    total_energy_loss += binary_collision_result.recoil_energy;

                    //total_deflection_angle += psi;
                    total_asymptotic_deflection += binary_collision_result.asymptotic_deflection;

                    //Rotate particle 1, 2 by lab frame scattering angles
                    particle::rotate_particle(&mut particle, binary_collision_result.psi,
                        binary_collision_geometries[k].phi_azimuthal);

                    particle::rotate_particle(&mut particle_2, -binary_collision_result.psi_recoil,
                        binary_collision_geometries[k].phi_azimuthal);
                    particle_2.dir_old.x = particle_2.dir.x;
                    particle_2.dir_old.y = particle_2.dir.y;
                    particle_2.dir_old.z = particle_2.dir.z;

                    //Only track number of strong collisions, i.e., k = 0
                    if (binary_collision_result.psi > 0.) & (k == 0) {
                        particle.number_collision_events += 1;
                    }

                    //Deep recoil suppression
                    //See Eckstein 1991 7.5.3 for recoil suppression function
                    if options.track_recoils & options.suppress_deep_recoils {
                        let E = particle.E;
                        let Za: f64 = particle.Z;
                        let Zb: f64 = particle_2.Z;

                        let Ma: f64 = particle.m;
                        let Mb: f64 = particle_2.m;

                        let n = material.total_number_density(particle_2.pos.x, particle_2.pos.y);
                        let a: f64 = interactions::screening_length(Za, Zb, options.interaction_potential);
                        let reduced_energy: f64 = LINDHARD_REDUCED_ENERGY_PREFACTOR*a*Mb/(Ma+Mb)/Za/Zb*E;
                        let estimated_range_of_recoils = (reduced_energy.powf(0.3) + 0.1).powf(3.)/n/a/a;

                        if let Closest::SinglePoint(p2) = material.closest_point(particle_2.pos.x, particle_2.pos.y) {
                            let dx = p2.x() - particle_2.pos.x;
                            let dy = p2.y() - particle_2.pos.y;
                            let distance_to_surface = (dx*dx + dy*dy).sqrt();

                            if (distance_to_surface < estimated_range_of_recoils) & (particle_2.E > particle_2.Ec) {
                                particles.push(particle_2);
                            }
                        }
                    //If transferred energy > cutoff energy, add recoil to particle vector
                    } else if options.track_recoils & (particle_2.E > particle_2.Ec) {
                        particles.push(particle_2);
                    }
                }
            }

            //Advance particle in space and track total distance traveled
            let distance_traveled = particle::particle_advance(&mut particle,
                binary_collision_geometries[0].mfp, total_asymptotic_deflection);

            //Subtract total energy from all simultaneous collisions and electronic stopping
            bca::update_particle_energy(&mut particle, &material, distance_traveled,
                total_energy_loss, distance_of_closest_approach, strong_collision_Z,
                strong_collision_index, &options);

            //Check boundary conditions on leaving and stopping
            material::boundary_condition_2D_planar(&mut particle, &material);

            //Set particle index to topmost particle
            particle_index = particles.len();
        }

        //Once particle finishes, begin data output
        //Stream current particle output to files
        //Incident particle, left material: reflected
        if particle.incident & particle.left {
            writeln!(
                reflected_file_stream, "{},{},{},{},{},{},{},{},{},{}",
                particle.m/mass_unit, particle.Z, particle.E/energy_unit,
                particle.pos.x/length_unit, particle.pos.y/length_unit, particle.pos.z/length_unit,
                particle.dir.x, particle.dir.y, particle.dir.z,
                particle.number_collision_events
            ).expect("Output error: could not write to reflected.output.");

        }

        //Incident particle, stopped in material: deposited
        if particle.incident & particle.stopped {
            writeln!(
                deposited_file_stream, "{},{},{},{},{},{}",
                particle.m/mass_unit, particle.Z,
                particle.pos.x/length_unit, particle.pos.y/length_unit, particle.pos.z/length_unit,
                particle.number_collision_events
            ).expect("Output error: could not write to deposited.output.");
        }

        //Not an incident particle, left material: sputtered
        if !particle.incident & particle.left {
            writeln!(
                sputtered_file_stream, "{},{},{},{},{},{},{},{},{},{},{},{},{}",
                particle.m/mass_unit, particle.Z, particle.E/energy_unit,
                particle.pos.x/length_unit, particle.pos.y/length_unit, particle.pos.z/length_unit,
                particle.dir.x, particle.dir.y, particle.dir.z,
                particle.number_collision_events,
                particle.pos_origin.x/length_unit, particle.pos_origin.y/length_unit, particle.pos_origin.z/length_unit
            ).expect("Output error: could not write to sputtered.output.");
        }

        //Trajectory output
        if particle.track_trajectories {
            writeln!(trajectory_data_stream, "{}", particle.trajectory.len())
                .expect("Output error: could not write trajectory length data.");

            for pos in particle.trajectory {
                writeln!(
                    trajectory_file_stream, "{},{},{},{},{},{}",
                    particle.m/mass_unit, particle.Z, pos.E/energy_unit,
                    pos.x/length_unit, pos.y/length_unit, pos.z/length_unit,
                ).expect("Output error: could not write to trajectories.output.");
            }
        }
    }
    //Flush all file streams before dropping to ensure all data is written
    reflected_file_stream.flush().unwrap();
    deposited_file_stream.flush().unwrap();
    sputtered_file_stream.flush().unwrap();
    trajectory_data_stream.flush().unwrap();
    trajectory_file_stream.flush().unwrap();
}

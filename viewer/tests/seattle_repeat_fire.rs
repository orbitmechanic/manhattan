//! S3.3d: 20-shot monotone accumulation on a CityPack Seattle record.
//! Ports the repeat_fire.rs assertions, parameterized by the dataset record.

use std::time::Duration;

use bevy::prelude::*;
use destructibles_bevy::alarm::AlarmState;
use destructibles_bevy::assets::WorldAssets;
use destructibles_bevy::audio::{AudioState, AudioVoicePool};
use destructibles_bevy::breach::{BreachState, DirtyShells};
use destructibles_bevy::building::{
    BuildingDetail, Buildings, DetailJobs, advance_detail, begin_detail_upgrade,
    spawn_building_world_lod,
};
use destructibles_bevy::collapse::Collapses;
use destructibles_bevy::damage::{CommandScopes, DamageJournal};
use destructibles_bevy::effects::ParticleBudget;
use destructibles_bevy::hazards::{HazardPolicy, Hazards};
use destructibles_bevy::impact::{Admission, PendingImpact, apply_impact, flush_pending_impact};
use destructibles_bevy::rewind::{RewindGuard, RewindState, building_checksum};
use destructibles_bevy::shard_rain::ShardBudget;
use destructibles_bevy::shudder::ShakeState;
use destructibles_bevy::streaming::Streaming;
use destructibles_bevy::streaming::{PristineChecksums, PristineParts};
use destructibles_bevy::structure::Structures;
use destructibles_core::family::MaterialFamily;
use destructibles_core::{BuildingSpec, FacadeStyle, ImpactSpec, core_for_style};
use manhattan_viewer::citypack::CityPack;

fn world() -> World {
    let mut world = World::new();
    world.insert_resource(WorldAssets::default());
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<StandardMaterial>>();
    world.init_resource::<Assets<AudioSource>>();
    world.insert_resource(Buildings::default());
    world.insert_resource(DetailJobs::default());
    world.insert_resource(Structures::default());
    world.insert_resource(Streaming::default());
    world.insert_resource(DamageJournal::default());
    world.insert_resource(CommandScopes::default());
    world.insert_resource(RewindState::default());
    world.insert_resource(RewindGuard::default());
    world.insert_resource(Admission::default());
    world.insert_resource(AudioVoicePool::default());
    world.insert_resource(AudioState::default());
    world.insert_resource(ShakeState::default());
    world.insert_resource(BreachState::default());
    world.insert_resource(DirtyShells::default());
    world.insert_resource(ShardBudget::default());
    world.insert_resource(ParticleBudget::default());
    world.insert_resource(PendingImpact::default());
    world.insert_resource(AlarmState::default());
    world.insert_resource(HazardPolicy::default());
    world.insert_resource(Hazards::default());
    world.insert_resource(Collapses::default());
    world.insert_resource(PristineChecksums::default());
    world.insert_resource(PristineParts::default());
    world.insert_resource(destructibles_bevy::rewind::SimClock { time: 10.0 });
    world.insert_resource(Time::<()>::default());
    world
}

#[test]
fn twenty_shots_accumulate_monotonically_on_a_seattle_record() {
    let pack = CityPack::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/datasets/seattle.citypack"),
    )
    .expect("citypack");
    let index = pack
        .neighborhoods()
        .into_iter()
        .find(|index| pack.nodes[*index].buildings > 0)
        .expect("neighborhood with buildings");
    let chunk = pack.read_chunk(index).expect("chunk");
    let building = chunk.buildings.first().expect("building");
    let id = building.id;
    let style = FacadeStyle::BrickMidrise;
    let footprint = building
        .perimeter
        .iter()
        .map(|point| {
            [
                point[0] - building.centroid[0],
                point[1] - building.centroid[1],
            ]
        })
        .collect::<Vec<_>>();
    let spec = BuildingSpec {
        family: MaterialFamily::MasonryStone,
        footprint: footprint.clone(),
        floor_height: building.floor_height_m,
        floor_count: building.floor_count.max(1) as u32,
        facade: style.material(),
        facade_style: style,
        windows: style.windows(),
        core: core_for_style(style, &footprint, id),
    };
    spec.validate().expect("valid Seattle spec");

    let mut world = world();
    spawn_building_world_lod(
        &mut world,
        id,
        spec,
        Vec3::new(
            building.centroid[0],
            building.base_elevation_m,
            building.centroid[1],
        ),
        BuildingDetail::Shell,
    );
    assert_eq!(
        world.resource::<Structures>().iter().count(),
        0,
        "shell spawned a skeleton"
    );
    assert!(begin_detail_upgrade(
        &mut world,
        id,
        destructibles_bevy::building::DetailTier::Structural
    ));
    for _ in 0..80 {
        if advance_detail(&mut world, id, 50.0)
            >= destructibles_bevy::building::DetailTier::Structural
        {
            break;
        }
    }
    assert_eq!(world.resource::<Structures>().iter().count(), 1);

    let impact = ImpactSpec {
        position: [
            building.centroid[0],
            building.base_elevation_m + 6.0,
            building.centroid[1] + 6.0,
        ],
        radius: 0.0,
        velocity: [0.0, -30.0, -300.0],
        mass_kg: 5000.0,
        explosive_yield_kg_tnt: 8.0,
    };
    let pristine = world
        .resource::<PristineChecksums>()
        .0
        .get(&id)
        .copied()
        .expect("pristine checksum");

    let mut last_len = 0usize;
    let mut last_head = 0.0f64;
    for shot in 0..20 {
        world
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(16));
        world
            .resource_mut::<destructibles_bevy::rewind::SimClock>()
            .time += 0.2;
        apply_impact(&mut world, id, impact);
        for _ in 0..4 {
            flush_pending_impact(&mut world);
        }
        let journal = world.resource::<DamageJournal>().0.clone();
        let len = journal.len();
        let head = journal.head_for(id);
        assert!(
            len >= last_len,
            "journal shrank after shot {shot}: {last_len} -> {len}"
        );
        assert!(
            head >= last_head - 1e-9,
            "journal head went backwards after shot {shot}: {last_head} -> {head}"
        );
        last_len = len;
        last_head = head;
        assert!(
            world.resource::<RewindState>().restoring.is_empty(),
            "restoration leaked into impact path at shot {shot}"
        );
    }
    assert!(last_len > 0, "no damage recorded");
    let journal = world.resource::<DamageJournal>().0.clone();
    assert_eq!(journal.entries_for(id).count(), last_len);
    let damaged = building_checksum(&mut world, id);
    assert_ne!(damaged, pristine, "damage did not move the checksum");
    println!("SEATTLE MONOTONE id {id} journal_len {last_len} head {last_head:.2}");
}

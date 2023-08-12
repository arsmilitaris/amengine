//! Contains APIs for ordering systems and executing them on a [`World`](crate::world::World)

mod condition;
mod config;
mod executor;
mod graph_utils;
#[allow(clippy::module_inception)]
mod schedule;
mod set;
mod state;

pub use self::condition::*;
pub use self::config::*;
pub use self::executor::*;
use self::graph_utils::*;
pub use self::schedule::*;
pub use self::set::*;
pub use self::state::*;

pub use self::graph_utils::NodeId;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    pub use crate as bevy_ecs;
    pub use crate::schedule::{IntoSystemSetConfig, Schedule, SystemSet};
    pub use crate::system::{Res, ResMut};
    pub use crate::{prelude::World, system::Resource};

    #[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
    enum TestSet {
        A,
        B,
        C,
        D,
        X,
    }

    #[derive(Resource, Default)]
    struct SystemOrder(Vec<u32>);

    #[derive(Resource, Default)]
    struct RunConditionBool(pub bool);

    #[derive(Resource, Default)]
    struct Counter(pub AtomicU32);

    fn make_exclusive_system(tag: u32) -> impl FnMut(&mut World) {
        move |world| world.resource_mut::<SystemOrder>().0.push(tag)
    }

    fn make_function_system(tag: u32) -> impl FnMut(ResMut<SystemOrder>) {
        move |mut resource: ResMut<SystemOrder>| resource.0.push(tag)
    }

    fn named_system(mut resource: ResMut<SystemOrder>) {
        resource.0.push(u32::MAX);
    }

    fn named_exclusive_system(world: &mut World) {
        world.resource_mut::<SystemOrder>().0.push(u32::MAX);
    }

    fn counting_system(counter: Res<Counter>) {
        counter.0.fetch_add(1, Ordering::Relaxed);
    }

    mod system_execution {
        use super::*;

        #[test]
        fn run_system() {
            let mut world = World::default();
            let mut schedule = Schedule::default();

            world.init_resource::<SystemOrder>();

            schedule.add_systems(make_function_system(0));
            schedule.run(&mut world);

            assert_eq!(world.resource::<SystemOrder>().0, vec![0]);
        }

        #[test]
        fn run_exclusive_system() {
            let mut world = World::default();
            let mut schedule = Schedule::default();

            world.init_resource::<SystemOrder>();

            schedule.add_systems(make_exclusive_system(0));
            schedule.run(&mut world);

            assert_eq!(world.resource::<SystemOrder>().0, vec![0]);
        }

        #[test]
        #[cfg(not(miri))]
        fn parallel_execution() {
            use bevy_tasks::{ComputeTaskPool, TaskPool};
            use std::sync::{Arc, Barrier};

            let mut world = World::default();
            let mut schedule = Schedule::default();
            let thread_count = ComputeTaskPool::init(TaskPool::default).thread_num();

            let barrier = Arc::new(Barrier::new(thread_count));

            for _ in 0..thread_count {
                let inner = barrier.clone();
                schedule.add_systems(move || {
                    inner.wait();
                });
            }

            schedule.run(&mut world);
        }
    }

    mod system_ordering {
        use super::*;

        #[test]
        fn order_systems() {
            let mut world = World::default();
            let mut schedule = Schedule::default();

            world.init_resource::<SystemOrder>();

            schedule.add_systems((
                named_system,
                make_function_system(1).before(named_system),
                make_function_system(0)
                    .after(named_system)
                    .in_set(TestSet::A),
            ));
            schedule.run(&mut world);

            assert_eq!(world.resource::<SystemOrder>().0, vec![1, u32::MAX, 0]);

            world.insert_resource(SystemOrder::default());

            assert_eq!(world.resource::<SystemOrder>().0, vec![]);

            // modify the schedule after it's been initialized and test ordering with sets
            schedule.configure_set(TestSet::A.after(named_system));
            schedule.add_systems((
                make_function_system(3)
                    .before(TestSet::A)
                    .after(named_system),
                make_function_system(4).after(TestSet::A),
            ));
            schedule.run(&mut world);

            assert_eq!(
                world.resource::<SystemOrder>().0,
                vec![1, u32::MAX, 3, 0, 4]
            );
        }

        #[test]
        fn order_exclusive_systems() {
            let mut world = World::default();
            let mut schedule = Schedule::default();

            world.init_resource::<SystemOrder>();

            schedule.add_systems((
                named_exclusive_system,
                make_exclusive_system(1).before(named_exclusive_system),
                make_exclusive_system(0).after(named_exclusive_system),
            ));
            schedule.run(&mut world);

            assert_eq!(world.resource::<SystemOrder>().0, vec![1, u32::MAX, 0]);
        }

        #[test]
        fn add_systems_correct_order() {
            let mut world = World::new();
            let mut schedule = Schedule::new();

            world.init_resource::<SystemOrder>();

            schedule.add_systems(
                (
                    make_function_system(0),
                    make_function_system(1),
                    make_exclusive_system(2),
                    make_function_system(3),
                )
                    .chain(),
            );

            schedule.run(&mut world);
            assert_eq!(world.resource::<SystemOrder>().0, vec![0, 1, 2, 3]);
        }

        #[test]
        fn add_systems_correct_order_nested() {
            let mut world = World::new();
            let mut schedule = Schedule::new();

            world.init_resource::<SystemOrder>();

            schedule.add_systems(
                (
                    (make_function_system(0), make_function_system(1)).chain(),
                    make_function_system(2),
                    (make_function_system(3), make_function_system(4)).chain(),
                    (
                        make_function_system(5),
                        (make_function_system(6), make_function_system(7)),
                    ),
                    (
                        (make_function_system(8), make_function_system(9)).chain(),
                        make_function_system(10),
                    ),
                )
                    .chain(),
            );

            schedule.run(&mut world);
            let order = &world.resource::<SystemOrder>().0;
            assert_eq!(
                &order[0..5],
                &[0, 1, 2, 3, 4],
                "first five items should be exactly ordered"
            );
            let unordered = &order[5..8];
            assert!(
                unordered.contains(&5) && unordered.contains(&6) && unordered.contains(&7),
                "unordered must be 5, 6, and 7 in any order"
            );
            let partially_ordered = &order[8..11];
            assert!(
                partially_ordered == [8, 9, 10] || partially_ordered == [10, 8, 9],
                "partially_ordered must be [8, 9, 10] or [10, 8, 9]"
            );
            assert!(order.len() == 11, "must have exacty 11 order entries");
        }
    }

    mod conditions {
        use crate::change_detection::DetectChanges;

        use super::*;

        #[test]
        fn system_with_condition() {
            let mut world = World::default();
            let mut schedule = Schedule::default();

            world.init_resource::<RunConditionBool>();
            world.init_resource::<SystemOrder>();

            schedule.add_systems(
                make_function_system(0).run_if(|condition: Res<RunConditionBool>| condition.0),
            );

            schedule.run(&mut world);
            assert_eq!(world.resource::<SystemOrder>().0, vec![]);

            world.resource_mut::<RunConditionBool>().0 = true;
            schedule.run(&mut world);
            assert_eq!(world.resource::<SystemOrder>().0, vec![0]);
        }

        #[test]
        fn systems_with_distributive_condition() {
            let mut world = World::default();
            let mut schedule = Schedule::default();

            world.insert_resource(RunConditionBool(true));
            world.init_resource::<SystemOrder>();

            fn change_condition(mut condition: ResMut<RunConditionBool>) {
                condition.0 = false;
            }

            schedule.add_systems(
                (
                    make_function_system(0),
                    change_condition,
                    make_function_system(1),
                )
                    .chain()
                    .distributive_run_if(|condition: Res<RunConditionBool>| condition.0),
            );

            schedule.run(&mut world);
            assert_eq!(world.resource::<SystemOrder>().0, vec![0]);
        }

        #[test]
        fn run_exclusive_system_with_condition() {
            let mut world = World::default();
            let mut schedule = Schedule::default();

            world.init_resource::<RunConditionBool>();
            world.init_resource::<SystemOrder>();

            schedule.add_systems(
                make_exclusive_system(0).run_if(|condition: Res<RunConditionBool>| condition.0),
            );

            schedule.run(&mut world);
            assert_eq!(world.resource::<SystemOrder>().0, vec![]);

            world.resource_mut::<RunConditionBool>().0 = true;
            schedule.run(&mut world);
            assert_eq!(world.resource::<SystemOrder>().0, vec![0]);
        }

        #[test]
        fn multiple_conditions_on_system() {
            let mut world = World::default();
            let mut schedule = Schedule::default();

            world.init_resource::<Counter>();

            schedule.add_systems((
                counting_system.run_if(|| false).run_if(|| false),
                counting_system.run_if(|| true).run_if(|| false),
                counting_system.run_if(|| false).run_if(|| true),
                counting_system.run_if(|| true).run_if(|| true),
            ));

            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);
        }

        #[test]
        fn multiple_conditions_on_system_sets() {
            let mut world = World::default();
            let mut schedule = Schedule::default();

            world.init_resource::<Counter>();

            schedule.configure_set(TestSet::A.run_if(|| false).run_if(|| false));
            schedule.add_systems(counting_system.in_set(TestSet::A));
            schedule.configure_set(TestSet::B.run_if(|| true).run_if(|| false));
            schedule.add_systems(counting_system.in_set(TestSet::B));
            schedule.configure_set(TestSet::C.run_if(|| false).run_if(|| true));
            schedule.add_systems(counting_system.in_set(TestSet::C));
            schedule.configure_set(TestSet::D.run_if(|| true).run_if(|| true));
            schedule.add_systems(counting_system.in_set(TestSet::D));

            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);
        }

        #[test]
        fn systems_nested_in_system_sets() {
            let mut world = World::default();
            let mut schedule = Schedule::default();

            world.init_resource::<Counter>();

            schedule.configure_set(TestSet::A.run_if(|| false));
            schedule.add_systems(counting_system.in_set(TestSet::A).run_if(|| false));
            schedule.configure_set(TestSet::B.run_if(|| true));
            schedule.add_systems(counting_system.in_set(TestSet::B).run_if(|| false));
            schedule.configure_set(TestSet::C.run_if(|| false));
            schedule.add_systems(counting_system.in_set(TestSet::C).run_if(|| true));
            schedule.configure_set(TestSet::D.run_if(|| true));
            schedule.add_systems(counting_system.in_set(TestSet::D).run_if(|| true));

            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);
        }

        #[test]
        fn system_conditions_and_change_detection() {
            #[derive(Resource, Default)]
            struct Bool2(pub bool);

            let mut world = World::default();
            world.init_resource::<Counter>();
            world.init_resource::<RunConditionBool>();
            world.init_resource::<Bool2>();
            let mut schedule = Schedule::default();

            schedule.add_systems(
                counting_system
                    .run_if(|res1: Res<RunConditionBool>| res1.is_changed())
                    .run_if(|res2: Res<Bool2>| res2.is_changed()),
            );

            // both resource were just added.
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);

            // nothing has changed
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);

            // RunConditionBool has changed, but counting_system did not run
            world.get_resource_mut::<RunConditionBool>().unwrap().0 = false;
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);

            // internal state for the bool2 run criteria was updated in the
            // previous run, so system still does not run
            world.get_resource_mut::<Bool2>().unwrap().0 = false;
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);

            // internal state for bool2 was updated, so system still does not run
            world.get_resource_mut::<RunConditionBool>().unwrap().0 = false;
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);

            // now check that it works correctly changing Bool2 first and then RunConditionBool
            world.get_resource_mut::<Bool2>().unwrap().0 = false;
            world.get_resource_mut::<RunConditionBool>().unwrap().0 = false;
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 2);
        }

        #[test]
        fn system_set_conditions_and_change_detection() {
            #[derive(Resource, Default)]
            struct Bool2(pub bool);

            let mut world = World::default();
            world.init_resource::<Counter>();
            world.init_resource::<RunConditionBool>();
            world.init_resource::<Bool2>();
            let mut schedule = Schedule::default();

            schedule.configure_set(
                TestSet::A
                    .run_if(|res1: Res<RunConditionBool>| res1.is_changed())
                    .run_if(|res2: Res<Bool2>| res2.is_changed()),
            );

            schedule.add_systems(counting_system.in_set(TestSet::A));

            // both resource were just added.
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);

            // nothing has changed
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);

            // RunConditionBool has changed, but counting_system did not run
            world.get_resource_mut::<RunConditionBool>().unwrap().0 = false;
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);

            // internal state for the bool2 run criteria was updated in the
            // previous run, so system still does not run
            world.get_resource_mut::<Bool2>().unwrap().0 = false;
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);

            // internal state for bool2 was updated, so system still does not run
            world.get_resource_mut::<RunConditionBool>().unwrap().0 = false;
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);

            // the system only runs when both are changed on the same run
            world.get_resource_mut::<Bool2>().unwrap().0 = false;
            world.get_resource_mut::<RunConditionBool>().unwrap().0 = false;
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 2);
        }

        #[test]
        fn mixed_conditions_and_change_detection() {
            #[derive(Resource, Default)]
            struct Bool2(pub bool);

            let mut world = World::default();
            world.init_resource::<Counter>();
            world.init_resource::<RunConditionBool>();
            world.init_resource::<Bool2>();
            let mut schedule = Schedule::default();

            schedule
                .configure_set(TestSet::A.run_if(|res1: Res<RunConditionBool>| res1.is_changed()));

            schedule.add_systems(
                counting_system
                    .run_if(|res2: Res<Bool2>| res2.is_changed())
                    .in_set(TestSet::A),
            );

            // both resource were just added.
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);

            // nothing has changed
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);

            // RunConditionBool has changed, but counting_system did not run
            world.get_resource_mut::<RunConditionBool>().unwrap().0 = false;
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);

            // we now only change bool2 and the system also should not run
            world.get_resource_mut::<Bool2>().unwrap().0 = false;
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);

            // internal state for the bool2 run criteria was updated in the
            // previous run, so system still does not run
            world.get_resource_mut::<RunConditionBool>().unwrap().0 = false;
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 1);

            // the system only runs when both are changed on the same run
            world.get_resource_mut::<Bool2>().unwrap().0 = false;
            world.get_resource_mut::<RunConditionBool>().unwrap().0 = false;
            schedule.run(&mut world);
            assert_eq!(world.resource::<Counter>().0.load(Ordering::Relaxed), 2);
        }
    }

    mod schedule_build_errors {
        use super::*;

        #[test]
        #[should_panic]
        fn dependency_loop() {
            let mut schedule = Schedule::new();
            schedule.configure_set(TestSet::X.after(TestSet::X));
        }

        #[test]
        fn dependency_cycle() {
            let mut world = World::new();
            let mut schedule = Schedule::new();

            schedule.configure_set(TestSet::A.after(TestSet::B));
            schedule.configure_set(TestSet::B.after(TestSet::A));

            let result = schedule.initialize(&mut world);
            assert!(matches!(result, Err(ScheduleBuildError::DependencyCycle)));

            fn foo() {}
            fn bar() {}

            let mut world = World::new();
            let mut schedule = Schedule::new();

            schedule.add_systems((foo.after(bar), bar.after(foo)));
            let result = schedule.initialize(&mut world);
            assert!(matches!(result, Err(ScheduleBuildError::DependencyCycle)));
        }

        #[test]
        #[should_panic]
        fn hierarchy_loop() {
            let mut schedule = Schedule::new();
            schedule.configure_set(TestSet::X.in_set(TestSet::X));
        }

        #[test]
        fn hierarchy_cycle() {
            let mut world = World::new();
            let mut schedule = Schedule::new();

            schedule.configure_set(TestSet::A.in_set(TestSet::B));
            schedule.configure_set(TestSet::B.in_set(TestSet::A));

            let result = schedule.initialize(&mut world);
            assert!(matches!(result, Err(ScheduleBuildError::HierarchyCycle)));
        }

        #[test]
        fn system_type_set_ambiguity() {
            // Define some systems.
            fn foo() {}
            fn bar() {}

            let mut world = World::new();
            let mut schedule = Schedule::new();

            // Schedule `bar` to run after `foo`.
            schedule.add_systems((foo, bar.after(foo)));

            // There's only one `foo`, so it's fine.
            let result = schedule.initialize(&mut world);
            assert!(result.is_ok());

            // Schedule another `foo`.
            schedule.add_systems(foo);

            // When there are multiple instances of `foo`, dependencies on
            // `foo` are no longer allowed. Too much ambiguity.
            let result = schedule.initialize(&mut world);
            assert!(matches!(
                result,
                Err(ScheduleBuildError::SystemTypeSetAmbiguity(_))
            ));

            // same goes for `ambiguous_with`
            let mut schedule = Schedule::new();
            schedule.add_systems(foo);
            schedule.add_systems(bar.ambiguous_with(foo));
            let result = schedule.initialize(&mut world);
            assert!(result.is_ok());
            schedule.add_systems(foo);
            let result = schedule.initialize(&mut world);
            assert!(matches!(
                result,
                Err(ScheduleBuildError::SystemTypeSetAmbiguity(_))
            ));
        }

        #[test]
        #[should_panic]
        fn configure_system_type_set() {
            fn foo() {}
            let mut schedule = Schedule::new();
            schedule.configure_set(foo.into_system_set());
        }

        #[test]
        fn hierarchy_redundancy() {
            let mut world = World::new();
            let mut schedule = Schedule::new();

            schedule.set_build_settings(ScheduleBuildSettings {
                hierarchy_detection: LogLevel::Error,
                ..Default::default()
            });

            // Add `A`.
            schedule.configure_set(TestSet::A);

            // Add `B` as child of `A`.
            schedule.configure_set(TestSet::B.in_set(TestSet::A));

            // Add `X` as child of both `A` and `B`.
            schedule.configure_set(TestSet::X.in_set(TestSet::A).in_set(TestSet::B));

            // `X` cannot be the `A`'s child and grandchild at the same time.
            let result = schedule.initialize(&mut world);
            assert!(matches!(
                result,
                Err(ScheduleBuildError::HierarchyRedundancy)
            ));
        }

        #[test]
        fn cross_dependency() {
            let mut world = World::new();
            let mut schedule = Schedule::new();

            // Add `B` and give it both kinds of relationships with `A`.
            schedule.configure_set(TestSet::B.in_set(TestSet::A));
            schedule.configure_set(TestSet::B.after(TestSet::A));
            let result = schedule.initialize(&mut world);
            assert!(matches!(
                result,
                Err(ScheduleBuildError::CrossDependency(_, _))
            ));
        }

        #[test]
        fn sets_have_order_but_intersect() {
            let mut world = World::new();
            let mut schedule = Schedule::new();

            fn foo() {}

            // Add `foo` to both `A` and `C`.
            schedule.add_systems(foo.in_set(TestSet::A).in_set(TestSet::C));

            // Order `A -> B -> C`.
            schedule.configure_sets((
                TestSet::A,
                TestSet::B.after(TestSet::A),
                TestSet::C.after(TestSet::B),
            ));

            let result = schedule.initialize(&mut world);
            // `foo` can't be in both `A` and `C` because they can't run at the same time.
            assert!(matches!(
                result,
                Err(ScheduleBuildError::SetsHaveOrderButIntersect(_, _))
            ));
        }

        #[test]
        fn ambiguity() {
            #[derive(Resource)]
            struct X;

            fn res_ref(_x: Res<X>) {}
            fn res_mut(_x: ResMut<X>) {}

            let mut world = World::new();
            let mut schedule = Schedule::new();

            schedule.set_build_settings(ScheduleBuildSettings {
                ambiguity_detection: LogLevel::Error,
                ..Default::default()
            });

            schedule.add_systems((res_ref, res_mut));
            let result = schedule.initialize(&mut world);
            assert!(matches!(result, Err(ScheduleBuildError::Ambiguity)));
        }
    }
}
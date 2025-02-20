// Copyright 2019 Twitter, Inc.
// Licensed under the Apache License, Version 2.0
// http://www.apache.org/licenses/LICENSE-2.0

mod event;

pub use self::event::PerfStatistic;
use crate::stats::{record_counter, register_counter};
use failure::Error;

use crate::common::*;
use crate::config::Config;
use crate::samplers::Sampler;

use logger::*;
use metrics::*;
use perfcnt::AbstractPerfCounter;
use perfcnt::PerfCounter;
use time;

use std::collections::HashMap;

pub struct Perf<'a> {
    config: &'a Config,
    counters: HashMap<PerfStatistic, Vec<PerfCounter>>,
    initialized: bool,
    recorder: &'a Recorder<AtomicU32>,
}

impl<'a> Sampler<'a> for Perf<'a> {
    fn new(
        config: &'a Config,
        recorder: &'a Recorder<AtomicU32>,
    ) -> Result<Option<Box<Self>>, Error> {
        if config.perf().enabled() {
            let mut counters = HashMap::new();
            let cores = hardware_threads().unwrap_or(1);

            for statistic in config.perf().statistics() {
                let mut event_counters = Vec::new();
                for core in 0..cores {
                    match statistic
                        .builder()
                        .on_cpu(core as isize)
                        .for_all_pids()
                        .finish()
                    {
                        Ok(c) => event_counters.push(c),
                        Err(e) => {
                            debug!("Failed to create PerfCounter for {:?}: {}", statistic, e);
                            break;
                        }
                    }
                }
                if event_counters.len() as u64 == cores {
                    trace!("Initialized PerfCounters for {:?}", statistic);
                    counters.insert(*statistic, event_counters);
                }
            }

            Ok(Some(Box::new(Self {
                config,
                counters,
                initialized: false,
                recorder,
            })))
        } else {
            Ok(None)
        }
    }

    fn name(&self) -> String {
        "perf".to_string()
    }

    fn sample(&mut self) -> Result<(), ()> {
        trace!("sample {}", self.name());
        let time = time::precise_time_ns();
        let mut current = HashMap::new();
        trace!("sampling: {} perf counters", self.counters.keys().count());
        for (event, counters) in &mut self.counters {
            let mut c = Vec::new();
            for counter in counters {
                let count = match counter.read() {
                    Ok(c) => c,
                    Err(e) => {
                        debug!("Could not read perf counter for event {:?}: {}", event, e);
                        0
                    }
                };
                c.push(count);
            }
            current.insert(*event, c);
        }
        if !self.initialized {
            self.register();
        }
        for statistic in self.counters.keys() {
            if let Some(counter) = current.get(statistic) {
                let value: u64 = counter.iter().sum();
                record_counter(self.recorder, statistic, time, value);
            }
        }
        Ok(())
    }

    fn register(&mut self) {
        trace!("register {}", self.name());
        if !self.initialized {
            for statistic in self.counters.keys() {
                register_counter(
                    self.recorder,
                    statistic,
                    TRILLION,
                    3,
                    self.config.general().window(),
                    PERCENTILES,
                );
            }
            self.initialized = true;
        }
    }

    fn deregister(&mut self) {
        trace!("deregister {}", self.name());
        if self.initialized {
            for statistic in self.counters.keys() {
                self.recorder.delete_channel(statistic.to_string());
            }
            self.initialized = false;
        }
    }
}

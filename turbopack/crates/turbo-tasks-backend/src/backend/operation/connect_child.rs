use serde::{Deserialize, Serialize};
use turbo_tasks::TaskId;

use super::{
    aggregation_update::{AggregationUpdateJob, AggregationUpdateQueue},
    ExecuteContext, Operation,
};
use crate::{
    data::{CachedDataItem, CachedDataItemKey},
    get, get_many,
};

#[derive(Serialize, Deserialize, Clone, Default)]
pub enum ConnectChildOperation {
    UpdateAggregation {
        task_id: TaskId,
        aggregation_update: AggregationUpdateQueue,
    },
    ScheduleTask {
        task_id: TaskId,
    },
    #[default]
    Done,
    // TODO Add aggregated edge
}

impl ConnectChildOperation {
    pub fn run(parent_task_id: TaskId, child_task_id: TaskId, ctx: ExecuteContext<'_>) {
        let mut parent_task = ctx.task(parent_task_id);
        if parent_task.add(CachedDataItem::Child {
            task: child_task_id,
            value: (),
        }) {
            // Update the task aggregation
            let mut queue = AggregationUpdateQueue::new();
            if get!(parent_task, AggregationNumber).is_some() {
                queue.push(AggregationUpdateJob::InnerHasNewFollower {
                    upper_ids: vec![parent_task_id],
                    new_follower_id: child_task_id,
                });
            } else {
                let upper_ids = get_many!(parent_task, Upper { task } => task);
                queue.push(AggregationUpdateJob::InnerHasNewFollower {
                    upper_ids,
                    new_follower_id: child_task_id,
                });
            }
            drop(parent_task);
            ConnectChildOperation::UpdateAggregation {
                task_id: child_task_id,
                aggregation_update: queue,
            }
            .execute(&ctx);
        }
    }
}

impl Operation for ConnectChildOperation {
    fn execute(mut self, ctx: &ExecuteContext<'_>) {
        loop {
            ctx.operation_suspend_point(&self);
            match self {
                ConnectChildOperation::UpdateAggregation {
                    task_id,
                    ref mut aggregation_update,
                } => {
                    if aggregation_update.process(ctx) {
                        // TODO check for active
                        self = ConnectChildOperation::ScheduleTask { task_id }
                    }
                }
                ConnectChildOperation::ScheduleTask { task_id } => {
                    {
                        let mut task = ctx.task(task_id);
                        task.add(CachedDataItem::new_scheduled(task_id));
                    }
                    ctx.schedule(task_id);

                    self = ConnectChildOperation::Done;
                }

                ConnectChildOperation::Done => {
                    return;
                }
            }
        }
    }
}

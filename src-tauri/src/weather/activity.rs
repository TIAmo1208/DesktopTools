use super::{City, Source, WeatherState};
use std::collections::BTreeMap;
use std::sync::{Condvar, LazyLock, Mutex};
use tauri::{AppHandle, Emitter, Listener, Manager};

/// 一个已展开且可见的天气格子登记，订阅身份不随页面重载复用。
struct Consumer
{
    monitor: String, // 所属原生宿主。
    source: Source,  // 格子自己选择的天气来源。
    city: City,      // 仅保存当前活动实例的位置。
}

/// 所有位置共享一个等待线程；无消费者时不保留周期唤醒。
#[derive(Default)]
struct Schedule
{
    next_id: u64,                       // 独立订阅身份。
    consumers: BTreeMap<u64, Consumer>, // 多屏消费者共享同一来源同一位置的请求。
    deadlines: BTreeMap<String, i64>,   // 毫秒时间，最大值表示等待配置或活动事件。
}

static SCHEDULE: LazyLock<(Mutex<Schedule>, Condvar)> = LazyLock::new(|| (Mutex::new(Schedule::default()), Condvar::new())); // 复用宿主时间源的条件等待模式。

/// 活动调度按来源与城市识别一次请求，同名城市跨来源互不共享。
fn plan_key(source: Source, city_id: &str) -> String { format!("{}:{}", source.key(), city_id) }

/// 命令层校验真实组件后登记活动位置。
pub(crate) fn subscribe(source: Source, monitor: String, city: City) -> u64
{
    let mut state = SCHEDULE.0.lock().unwrap_or_else(|error| error.into_inner());
    state.next_id += 1;
    let id = state.next_id; // 清理旧身份不会影响新订阅。
    state.consumers.insert(id, Consumer { monitor, source, city });
    SCHEDULE.1.notify_one();
    id
}

/// 解除一个活动实例并释放没有消费者的来源位置计划。
pub(crate) fn unsubscribe(id: u64)
{
    let mut state = SCHEDULE.0.lock().unwrap_or_else(|error| error.into_inner());
    state.consumers.remove(&id);
    let keys = state.consumers.values().map(|value| plan_key(value.source, &value.city.id)).collect::<Vec<_>>();
    state.deadlines.retain(|key, _| keys.contains(key));
    SCHEDULE.1.notify_one();
}

/// 宿主销毁或重载时清理失去页面清理回调的订阅。
pub(crate) fn clear_monitor(monitor: &str)
{
    let mut state = SCHEDULE.0.lock().unwrap_or_else(|error| error.into_inner());
    state.consumers.retain(|_, value| value.monitor != monitor);
    let keys = state.consumers.values().map(|value| plan_key(value.source, &value.city.id)).collect::<Vec<_>>();
    state.deadlines.retain(|key, _| keys.contains(key));
    SCHEDULE.1.notify_one();
}

/// 配置或宿主状态变化唤醒共享线程，不增加轮询。
fn wake(reset: bool)
{
    let mut state = SCHEDULE.0.lock().unwrap_or_else(|error| error.into_inner());
    if reset
    {
        state.deadlines.clear();
    }
    SCHEDULE.1.notify_one();
}

/// 网络开始前和返回后都重新判断活动实例与已保存功能开关。
fn active(app: &AppHandle, source: Source, city: &str) -> bool { app.state::<crate::app::settings::SettingsState>().preferences().is_ok_and(|value| value.weather_enabled) && SCHEDULE.0.lock().unwrap_or_else(|error| error.into_inner()).consumers.values().any(|value| value.source == source && value.city.id == city && !crate::platform::windows::refresh_policy::is_paused(&value.monitor)) }

/// 启动一个按截止时间等待的天气更新线程。
pub(crate) fn start(app: AppHandle)
{
    for name in ["weather-configuration-changed", "preferences-changed", "desktop-refresh-policy-changed"]
    {
        app.listen(name, move |_| wake(name == "weather-configuration-changed"));
    }
    std::thread::spawn(move || {
        loop
        {
            let mut schedule = SCHEDULE.0.lock().unwrap_or_else(|error| error.into_inner());
            let enabled = app.state::<crate::app::settings::SettingsState>().preferences().is_ok_and(|value| value.weather_enabled);
            let now = chrono::Utc::now().timestamp_millis(); // 每轮只为最近的活动来源位置设置一次等待。
            let next = schedule.consumers.values().filter(|value| enabled && !crate::platform::windows::refresh_policy::is_paused(&value.monitor)).map(|value| (schedule.deadlines.get(&plan_key(value.source, &value.city.id)).copied().unwrap_or(now), plan_key(value.source, &value.city.id), value.source, value.city.clone())).min_by_key(|(deadline, key, _, _)| (*deadline, key.clone())); // 同一截止时间按来源与城市固定顺序，避免计划顺序随注册次序漂移。
            let Some((deadline, key, source, city)) = next
            else
            {
                drop(SCHEDULE.1.wait(schedule).unwrap_or_else(|error| error.into_inner()));
                continue;
            };
            if deadline > now
            {
                drop(SCHEDULE.1.wait_timeout(schedule, std::time::Duration::from_millis((deadline - now) as u64)).unwrap_or_else(|error| error.into_inner()));
                continue;
            }
            schedule.deadlines.insert(key.clone(), i64::MAX);
            drop(schedule);
            let state = app.state::<WeatherState>();
            let result = state.refresh(source, city.clone(), || active(&app, source, &city.id)); // 同来源同位置共享请求，暂停后不启动下一个模块。
            if !active(&app, source, &city.id)
            {
                SCHEDULE.0.lock().unwrap_or_else(|error| error.into_inner()).deadlines.remove(&key);
                continue;
            }
            let value = result.unwrap_or_else(|error| {
                crate::log_warn!("天气调度：读取失败，来源={}，城市={}，原因={error}", source.key(), city.id);
                serde_json::json!({"city":city,"moduleErrors":{"current":error,"hourly":error,"daily":error}})
            });
            let next = state.next_refresh(source, &city, &value).unwrap_or_else(|_| Some(chrono::Utc::now().timestamp_millis() + 60_000));
            let mut schedule = SCHEDULE.0.lock().unwrap_or_else(|error| error.into_inner());
            // 配置事件清空的计划不能被旧任务覆盖；新一轮会重新读取当前配置。
            if schedule.deadlines.get(&key) != Some(&i64::MAX)
            {
                continue;
            }
            schedule.deadlines.insert(key, next.unwrap_or(i64::MAX));
            drop(schedule);
            let mut payload = value; // 事件载荷在预报结果上补充来源标识。
            if let Some(object) = payload.as_object_mut()
            {
                object.insert("source".to_string(), serde_json::json!(source.key())); // 同名城市跨来源各自更新，前端按来源与城市一起过滤。
            }
            if let Err(error) = app.emit("weather-forecast-changed", payload)
            {
                crate::log_warn!("天气调度：更新通知失败，原因={error}");
            }
        }
    });
}

#[cfg(test)]
mod tests
{
    use super::*;

    /// 同名城市样例；来源模块的详情链接规则不参与登记。
    fn city(id: &str) -> City { City { id: id.into(), name: id.into(), adm1: String::new(), adm2: String::new(), country: String::new(), lat: "0".into(), lon: "0".into(), tz: "UTC".into(), fx_link: String::new() } }

    /// 活动计划是进程级共享状态，两个用例合并执行以免互相清理同一份计划。
    #[test]
    fn subscriptions_clean_up_per_monitor_and_source()
    {
        let first = subscribe(Source::Qweather, "DISPLAY1".into(), city("activity-test-a"));
        let second = subscribe(Source::Qweather, "DISPLAY2".into(), city("activity-test-b"));
        assert_eq!(SCHEDULE.0.lock().unwrap().consumers.len(), 2);
        unsubscribe(first);
        assert_eq!(SCHEDULE.0.lock().unwrap().consumers.len(), 1);
        clear_monitor("DISPLAY2");
        assert!(SCHEDULE.0.lock().unwrap().consumers.is_empty());
        unsubscribe(second);

        let qweather = subscribe(Source::Qweather, "DISPLAY3".into(), city("activity-test-shared"));
        let foreca = subscribe(Source::Foreca, "DISPLAY3".into(), city("activity-test-shared")); // 同名城市跨来源各自登记。
        SCHEDULE.0.lock().unwrap().deadlines.insert(plan_key(Source::Foreca, "activity-test-shared"), i64::MAX);
        unsubscribe(qweather);
        assert!(SCHEDULE.0.lock().unwrap().deadlines.contains_key(&plan_key(Source::Foreca, "activity-test-shared"))); // 解除一家来源不影响另一家的同名城市计划。
        clear_monitor("DISPLAY3");
        assert!(!SCHEDULE.0.lock().unwrap().deadlines.contains_key(&plan_key(Source::Foreca, "activity-test-shared"))); // 显示器清理释放其全部计划。
        unsubscribe(foreca);
    }
}

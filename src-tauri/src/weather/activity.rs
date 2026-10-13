use super::{City, WeatherState};
use std::collections::BTreeMap;
use std::sync::{Condvar, LazyLock, Mutex};
use tauri::{AppHandle, Emitter, Listener, Manager};

/// 一个已展开且可见的天气格子登记，订阅身份不随页面重载复用。
struct Consumer
{
    monitor: String, // 所属原生宿主。
    city: City,      // 仅保存当前活动实例的位置。
}

/// 所有位置共享一个等待线程；无消费者时不保留周期唤醒。
#[derive(Default)]
struct Schedule
{
    next_id: u64,                       // 独立订阅身份。
    consumers: BTreeMap<u64, Consumer>, // 多屏消费者共享位置请求。
    deadlines: BTreeMap<String, i64>,   // 毫秒时间，最大值表示等待配置或活动事件。
}

static SCHEDULE: LazyLock<(Mutex<Schedule>, Condvar)> = LazyLock::new(|| (Mutex::new(Schedule::default()), Condvar::new())); // 复用宿主时间源的条件等待模式。

/// 命令层校验真实组件后登记活动位置。
pub(crate) fn subscribe(monitor: String, city: City) -> u64
{
    let mut state = SCHEDULE.0.lock().unwrap_or_else(|error| error.into_inner());
    state.next_id += 1;
    let id = state.next_id; // 清理旧身份不会影响新订阅。
    state.deadlines.remove(&city.id);
    state.consumers.insert(id, Consumer { monitor, city });
    SCHEDULE.1.notify_one();
    id
}

/// 解除一个活动实例并释放没有消费者的位置计划。
pub(crate) fn unsubscribe(id: u64)
{
    let mut state = SCHEDULE.0.lock().unwrap_or_else(|error| error.into_inner());
    state.consumers.remove(&id);
    let ids = state.consumers.values().map(|value| value.city.id.clone()).collect::<Vec<_>>();
    state.deadlines.retain(|city, _| ids.contains(city));
    SCHEDULE.1.notify_one();
}

/// 宿主销毁或重载时清理失去页面清理回调的订阅。
pub(crate) fn clear_monitor(monitor: &str)
{
    let mut state = SCHEDULE.0.lock().unwrap_or_else(|error| error.into_inner());
    state.consumers.retain(|_, value| value.monitor != monitor);
    let ids = state.consumers.values().map(|value| value.city.id.clone()).collect::<Vec<_>>();
    state.deadlines.retain(|city, _| ids.contains(city));
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
fn active(app: &AppHandle, city: &str) -> bool { app.state::<crate::app::settings::SettingsState>().preferences().is_ok_and(|value| value.weather_enabled) && SCHEDULE.0.lock().unwrap_or_else(|error| error.into_inner()).consumers.values().any(|value| value.city.id == city && !crate::platform::windows::refresh_policy::is_paused(&value.monitor)) }

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
            let now = chrono::Utc::now().timestamp_millis(); // 每轮只为最近的活动位置设置一次等待。
            let next = schedule.consumers.values().filter(|value| enabled && !crate::platform::windows::refresh_policy::is_paused(&value.monitor)).map(|value| (schedule.deadlines.get(&value.city.id).copied().unwrap_or(now), value.city.clone())).min_by_key(|(deadline, _)| *deadline);
            let Some((deadline, city)) = next
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
            schedule.deadlines.insert(city.id.clone(), i64::MAX);
            drop(schedule);
            let state = app.state::<WeatherState>();
            let result = state.refresh(city.clone(), || active(&app, &city.id)); // 同位置共享请求，暂停后不启动下一个模块。
            if !active(&app, &city.id)
            {
                SCHEDULE.0.lock().unwrap_or_else(|error| error.into_inner()).deadlines.remove(&city.id);
                continue;
            }
            let value = result.unwrap_or_else(|error| {
                crate::log_warn!("天气调度：读取失败，原因={error}");
                serde_json::json!({"city":city,"moduleErrors":{"current":error,"hourly":error,"daily":error}})
            });
            let next = state.next_refresh(&city, &value).unwrap_or_else(|_| Some(chrono::Utc::now().timestamp_millis() + 60_000));
            let mut schedule = SCHEDULE.0.lock().unwrap_or_else(|error| error.into_inner());
            // 配置事件清空的计划不能被旧任务覆盖；新一轮会重新读取当前配置。
            if schedule.deadlines.get(&city.id) != Some(&i64::MAX)
            {
                continue;
            }
            schedule.deadlines.insert(city.id.clone(), next.unwrap_or(i64::MAX));
            drop(schedule);
            if let Err(error) = app.emit("weather-forecast-changed", value)
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

    #[test]
    fn subscription_cleanup_keeps_other_city_active()
    {
        let city = |id: &str| City { id: id.into(), name: id.into(), adm1: String::new(), adm2: String::new(), country: String::new(), lat: "0".into(), lon: "0".into(), tz: "UTC".into(), fx_link: format!("https://www.qweather.com/weather/{id}-{id}.html") };
        let first = subscribe("DISPLAY1".into(), city("activity-test-a"));
        let second = subscribe("DISPLAY2".into(), city("activity-test-b"));
        assert_eq!(SCHEDULE.0.lock().unwrap().consumers.len(), 2);
        unsubscribe(first);
        assert_eq!(SCHEDULE.0.lock().unwrap().consumers.len(), 1);
        clear_monitor("DISPLAY2");
        assert!(SCHEDULE.0.lock().unwrap().consumers.is_empty());
        unsubscribe(second);
    }
}

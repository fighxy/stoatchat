// =============================================================================
// ФАЙЛ: crates/core/database/src/lib.rs
// НАЗНАЧЕНИЕ: Корневой модуль крейта revolt-database.
//   Здесь собраны: подключение внешних крейтов, определение макросов,
//   объявление внутренних модулей и реэкспорт публичного API.
//   Этот файл — "клей", соединяющий все части слоя работы с базой данных.
// =============================================================================

// `#[macro_use] extern crate serde` — подключаем serde с активацией всех его макросов.
// serde — де-факто стандарт для сериализации/десериализации данных в Rust.
// Используется для преобразования структур данных в JSON (для API) и BSON (для MongoDB).
#[macro_use]
extern crate serde;

// async_recursion позволяет писать рекурсивные async-функции.
// Нативно в Rust это невозможно из-за неизвестного размера Future,
// поэтому макрос оборачивает возвращаемое значение в Box<dyn Future>.
#[macro_use]
extern crate async_recursion;

// async_trait — аналогично крейту permissions (см. там пояснение).
// Позволяет использовать `async fn` в определениях трейтов.
#[macro_use]
extern crate async_trait;

// `log` — стандартный крейт для структурированного логирования в Rust.
// Предоставляет макросы: `info!()`, `warn!()`, `error!()`, `debug!()`, `trace!()`.
// Конкретный бэкенд вывода логов настраивается отдельно (в нашем случае — env_logger).
#[macro_use]
extern crate log;

// revolt_optional_struct — кастомный процедурный макрос этого проекта.
// Генерирует "partial"-версию структуры, где все поля становятся Option<T>.
// Это нужно для частичных обновлений в БД: PATCH /users/{id} обновляет только переданные поля.
#[macro_use]
extern crate revolt_optional_struct;

// revolt_result — крейт с общими типами ошибок (см. crates/core/result/).
// Импортируем с #[macro_use] чтобы использовать create_error! и create_database_error!.
#[macro_use]
extern crate revolt_result;

// Реэкспортируем iso8601_timestamp — крейт для работы с временными метками
// в формате ISO 8601. Используется для полей типа "дата создания", "дата редактирования".
pub use iso8601_timestamp;

// MongoDB-крейт подключается только если активирована feature "mongodb".
// В тестах используется Reference (in-memory) БД без MongoDB.
#[cfg(feature = "mongodb")]
pub use mongodb;

// BSON (Binary JSON) — формат, используемый MongoDB для хранения данных.
// Нужен только при работе с реальной MongoDB.
#[cfg(feature = "mongodb")]
#[macro_use]
extern crate bson;

// =============================================================================
// ПРОВЕРКА ОБЯЗАТЕЛЬНЫХ FEATURE FLAGS
// =============================================================================

// `compile_error!` — вызывает ошибку компиляции с пользовательским сообщением.
// Если async-std-runtime feature не включена, сборка падает немедленно
// с понятным сообщением — это лучше чем загадочные ошибки в runtime.
// async-std — асинхронный runtime, на котором работает этот крейт.
#[cfg(not(feature = "async-std-runtime"))]
compile_error!("async-std-runtime feature must be enabled.");

// =============================================================================
// МАКРОС query! — АБСТРАКЦИЯ НАД ОПЕРАЦИЯМИ С БАЗОЙ ДАННЫХ
// =============================================================================

// Макрос существует в двух вариантах: debug и release.
// Это пример условной компиляции для разного поведения в разных окружениях.

// DEBUG-вариант: используем unwrap() — при ошибке программа паникует.
// Это помогает при разработке: ошибка БД сразу видна как паника со стектрейсом.
// `#[macro_export]` — делает макрос доступным для пользователей крейта.
#[macro_export]
#[cfg(debug_assertions)]
macro_rules! query {
    // Синтаксис паттерна: `$self` — объект БД, `$type` — метод, `$collection` — коллекция,
    // `$($rest:expr),+` — один или более дополнительных аргументов.
    ( $self: ident, $type: ident, $collection: expr, $($rest:expr),+ ) => {
        Ok($self.$type($collection, $($rest),+).await.unwrap())
    };
}

// RELEASE-вариант: преобразуем ошибку в DatabaseError вместо паники.
// `capture_internal_error!` — отправляет ошибку в Sentry для мониторинга.
// `stringify!($type)` — макрос, превращающий идентификатор в строку при компиляции.
#[macro_export]
#[cfg(not(debug_assertions))]
macro_rules! query {
    ( $self: ident, $type: ident, $collection: expr, $($rest:expr),+ ) => {
        $self.$type($collection, $($rest),+).await
            .map_err(|err| {
                revolt_config::capture_internal_error!(err);
                create_database_error!(stringify!($type), $collection)
            })
    };
}

// =============================================================================
// ВСПОМОГАТЕЛЬНЫЕ МАКРОСЫ ДЛЯ DERIVE
// =============================================================================

// `database_derived!` — добавляет только трейт Clone к любой структуре/enum.
// Clone позволяет создать копию значения через `.clone()`.
// Используется для типов, которые нужно клонировать, но не сериализовать.
macro_rules! database_derived {
    ( $( $item:item )+ ) => {
        $(
            #[derive(Clone)]
            $item
        )+
    };
}

// `auto_derived!` — добавляет стандартный набор трейтов к моделям данных:
// - Serialize/Deserialize — для JSON/BSON сериализации
// - Debug — для отладочного вывода через {:?}
// - Clone — для создания копий
// - Eq, PartialEq — для сравнения значений через == и !=
macro_rules! auto_derived {
    ( $( $item:item )+ ) => {
        $(
            #[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
            $item
        )+
    };
}

// `auto_derived_partial!` — самый важный макрос этого крейта.
// Генерирует ДВЕ структуры из одного определения:
// 1. Оригинальную структуру с обычными полями (например, `Message`)
// 2. "Partial"-версию, где все поля обёрнуты в Option<T> (например, `PartialMessage`)
//
// `$name` — имя для генерируемой partial-структуры (передаётся как строка).
// `OptionalStruct` — процедурный макрос из revolt_optional_struct.
// `#[opt_skip_serializing_none]` — не включать None-поля в JSON при сериализации.
// `#[opt_some_priority]` — при слиянии данных Some(x) имеет приоритет над None.
//
// Зачем это нужно? При PATCH-запросах клиент присылает только изменённые поля.
// Partial-структура позволяет хранить "дельту" обновления и применять её к объекту.
macro_rules! auto_derived_partial {
    ( $item:item, $name:expr ) => {
        #[derive(OptionalStruct, Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
        #[optional_derive(Serialize, Deserialize, Debug, Clone, Default, Eq, PartialEq)]
        #[optional_name = $name]
        #[opt_skip_serializing_none]
        #[opt_some_priority]
        $item
    };
}

// =============================================================================
// МОДУЛИ
// =============================================================================

// `mod drivers` — объявление модуля без `pub`: drivers доступен только внутри крейта.
// `pub use drivers::*` — но его публичное содержимое реэкспортируется наружу.
// В drivers/ находятся: MongoDB-драйвер, Reference (in-memory) драйвер и enum Database.
mod drivers;
pub use drivers::*;

// =============================================================================
// ТЕСТОВЫЙ МАКРОС database_test!
// =============================================================================

// Этот макрос доступен только в тестах (`#[cfg(test)]`).
// Он автоматизирует бойлерплейт при написании тестов с базой данных:
// 1. Создаёт тестовую БД с уникальным именем (на основе файла и строки теста)
// 2. Очищает БД перед тестом
// 3. Запускает тестовую функцию
// 4. Очищает БД после теста (гарантия изоляции тестов)
//
// Пример использования:
//   database_test!(|db| async move {
//       let user = db.fetch_user("123").await?;
//       assert_eq!(user.username, "alice");
//   });
#[cfg(test)]
macro_rules! database_test {
    ( | $db: ident | $test:expr ) => {
        // DatabaseInfo::Test — вариант enum для тестовых БД.
        // Имя БД формируется из пути файла и номера строки — гарантирует уникальность
        // при параллельном запуске тестов (что делает cargo nextest).
        let db = $crate::DatabaseInfo::Test(format!(
            "{}:{}",
            file!().replace('/', "_").replace(".rs", ""),
            line!()
        ))
        .connect()
        .await
        .expect("Database connection failed.");

        // Очищаем БД перед тестом — на случай если предыдущий тест упал
        // и не успел убраться за собой.
        db.drop_database().await;

        // `#[allow(clippy::redundant_closure_call)]` — подавляем предупреждение линтера,
        // который видит паттерн `(|x| expr)(value)` и считает его избыточным.
        // Здесь это намеренный паттерн для передачи `$db` в тестовое замыкание.
        #[allow(clippy::redundant_closure_call)]
        (|$db: $crate::Database| $test)(db.clone()).await;

        // Очищаем БД после теста — изолируем тесты друг от друга.
        db.drop_database().await
    };
}

// `models` — объявление модуля со всеми моделями данных (User, Message, Channel и т.д.)
// `pub mod util` — публичный модуль с утилитами (вспомогательные функции, типы).
// `pub use models::*` — реэкспорт всех публичных типов моделей.
mod models;
pub mod util;
pub use models::*;

// `events` — модуль для системы событий в реальном времени (EventV1 и т.д.)
pub mod events;

// Фоновые задачи (tasks) — включаются только при активной feature "tasks".
// Это задачи типа: обработка embed-ссылок, подтверждение прочтения сообщений и др.
#[cfg(feature = "tasks")]
pub mod tasks;

// amqp — интеграция с RabbitMQ (брокер сообщений для push-уведомлений).
// AMQP — Advanced Message Queuing Protocol.
mod amqp;
pub use amqp::amqp::AMQP;

// Поддержка голосовых звонков через LiveKit — опциональная feature.
#[cfg(feature = "voice")]
pub mod voice;

// =============================================================================
// УТИЛИТАРНЫЕ ФУНКЦИИ
// =============================================================================

// Эти функции используются как предикаты для `#[serde(skip_serializing_if = "...")]`.
// serde позволяет задать функцию, которая решает, включать ли поле в JSON.
// Это уменьшает размер передаваемых данных — не отправляем лишние поля.

/// Возвращает true если значение равно false.
/// Используется для пропуска `false`-полей при сериализации.
pub fn if_false(t: &bool) -> bool {
    !t
}

/// Возвращает true если Option не равен Some(true).
/// Используется для пропуска полей, которые не являются явным `true`.
pub fn if_option_false(t: &Option<bool>) -> bool {
    t != &Some(true)
}

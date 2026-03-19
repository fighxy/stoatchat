// =============================================================================
// ФАЙЛ: crates/delta/src/main.rs
// НАЗНАЧЕНИЕ: Точка входа REST API сервера (delta).
//   Здесь инициализируются все зависимости и запускается Rocket-сервер на порту 14702.
//
// КЛЮЧЕВЫЕ КОНЦЕПТЫ:
// - Rocket — Rust веб-фреймворк с декларативными маршрутами
// - manage() — инъекция глобального состояния (DI-контейнер Rocket)
// - attach() — добавление middleware (fairing в терминологии Rocket)
// - AMQP/RabbitMQ — брокер сообщений для push-уведомлений
// - async_std::task::spawn — запуск фоновых задач
// =============================================================================

// `#[macro_use] extern crate rocket` — импорт Rocket с его макросами.
// Макросы Rocket (#[get], #[post], #[launch] и т.д.) — основа декларативного стиля фреймворка.
#[macro_use]
extern crate rocket;
// revolt_rocket_okapi — форк okapi (OpenAPI для Rocket).
// Предоставляет #[openapi] макрос и функции генерации OpenAPI-спецификации.
#[macro_use]
extern crate revolt_rocket_okapi;
// serde_json — для работы с JSON: парсинг, сериализация.
// `json!({...})` макрос создаёт JSON-значение прямо в коде.
#[macro_use]
extern crate serde_json;

// Маршруты REST API (handlers для всех эндпоинтов)
pub mod routes;
// Утилиты: rate limiting конфигурация, вспомогательные типы
pub mod util;

use revolt_config::config;
// EventV1 — перечисление всех событий реального времени (отправляются через WebSocket)
use revolt_database::events::client::EventV1;
// AMQP — обёртка над RabbitMQ-соединением для отправки push-уведомлений
use revolt_database::AMQP;
use revolt_ratelimits::rocket as ratelimiter;
// Основные типы Rocket
use rocket::{Build, Rocket};
// CorsOptions — настройка политики CORS (Cross-Origin Resource Sharing)
use rocket_cors::{AllowedOrigins, CorsOptions};
// PrometheusMetrics — middleware для сбора метрик (запросы/сек, латентность и т.д.)
use rocket_prometheus::PrometheusMetrics;
use std::net::Ipv4Addr;
use std::str::FromStr;

// amqprs — реализация AMQP 0.9.1 (протокол RabbitMQ) для Rust
use amqprs::{
    channel::ExchangeDeclareArguments,
    connection::{Connection, OpenConnectionArguments},
};
// `unbounded()` — создаёт канал связи между async-задачами без ограничения буфера.
// Возвращает пару (sender, receiver) — стандартный паттерн для межзадачного общения.
use async_std::channel::unbounded;
// authifier — крейт аутентификации: сессии, аккаунты
use authifier::AuthifierEvent;
use rocket::data::ToByteUnit;
// VoiceClient — клиент для LiveKit (голосовые звонки)
use revolt_database::voice::VoiceClient;

// =============================================================================
// ФУНКЦИЯ ИНИЦИАЛИЗАЦИИ WEB-СЕРВЕРА
// =============================================================================

// `pub async fn web()` — асинхронная функция, конфигурирующая и возвращающая
// экземпляр Rocket приложения. Вынесена отдельно для тестируемости.
// `-> Rocket<Build>` — возвращает настроенный но ещё не запущенный сервер.
pub async fn web() -> Rocket<Build> {
    // Загружаем конфигурацию из Revolt.toml (и опционально Revolt.overrides.toml)
    let config = config().await;

    // Проверяем обязательные переменные окружения и настройки
    config.preflight_checks();

    // Подключаемся к базе данных.
    // `DatabaseInfo::Auto` — автоматически выбирает тип БД по конфигурации.
    // `.unwrap()` — паникует если подключение не удалось (критическая ошибка при старте).
    let db = revolt_database::DatabaseInfo::Auto.connect().await.unwrap();
    log::info!("database_here {db:?}");
    // Применяем миграции схемы данных (создание индексов, обновление коллекций)
    db.migrate_database().await.unwrap();

    // `unbounded()` — создаёт асинхронный канал сообщений.
    // `_` вместо sender — мы не используем sender (он должен быть встроен в authifier),
    // но в текущей реализации он отброшен, что означает канал немедленно закрывается.
    let (_, receiver) = unbounded();

    // Инициализируем систему аутентификации (authifier).
    // `to_authifier()` создаёт конфигурацию authifier с нашей БД.
    let authifier = db.clone().to_authifier().await;

    // Запускаем фоновую задачу для обработки событий аутентификации.
    // `async_std::task::spawn` — запускает async-замыкание как отдельную задачу
    // (аналог thread::spawn но для async-кода, без создания нового потока ОС).
    async_std::task::spawn(async move {
        // Бесконечный цикл ожидания событий из канала.
        // `while let Ok(event) = receiver.recv().await` — получаем события пока канал открыт.
        // Когда sender закрыт (или dropped), recv() вернёт Err и цикл завершится.
        while let Ok(event) = receiver.recv().await {
            // Паттерн-матчинг по типу события authifier:
            // разные события → разные адресаты (глобальная рассылка или личное уведомление)
            match &event {
                // Создание сессии и аккаунта — рассылаем всем подписанным клиентам
                AuthifierEvent::CreateSession { .. } | AuthifierEvent::CreateAccount { .. } => {
                    EventV1::Auth(event).global().await
                }
                // Удаление сессии — рассылаем только конкретному пользователю
                // `user_id` извлекается из enum-варианта через деструктуризацию
                AuthifierEvent::DeleteSession { user_id, .. }
                | AuthifierEvent::DeleteAllSessions { user_id, .. } => {
                    let id = user_id.to_string();
                    EventV1::Auth(event).private(id).await
                }
            }
        }
    });

    // =============================================================================
    // КОНФИГУРАЦИЯ CORS
    // =============================================================================

    // CORS (Cross-Origin Resource Sharing) — механизм браузерной безопасности.
    // Без настройки CORS браузер блокирует запросы с одного домена к другому.
    // Здесь разрешаем запросы со всех источников (AllowedOrigins::All) —
    // приемлемо для публичного API, но не для production без дополнительной защиты.
    let cors = CorsOptions {
        allowed_origins: AllowedOrigins::All,
        // Перечисляем разрешённые HTTP-методы. `.map().collect()` — итераторный подход:
        // из массива строк создаём HashSet<Method> через FromStr-парсинг.
        allowed_methods: [
            "Get", "Put", "Post", "Delete", "Options", "Head", "Trace", "Connect", "Patch",
        ]
        .iter()
        .map(|s| FromStr::from_str(s).unwrap())
        .collect(),
        // Expose rate limit заголовки клиентам — они могут читать их в браузере.
        // По умолчанию браузер скрывает кастомные заголовки — нужно явно разрешить.
        expose_headers: [
            "X-Ratelimit-Limit",
            "X-Ratelimit-Bucket",
            "X-Ratelimit-Remaining",
            "X-Ratelimit-Reset-After",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        // `..Default::default()` — синтаксис "struct update": остальные поля
        // берём из Default-реализации CorsOptions.
        ..Default::default()
    }
    .to_cors()
    .expect("Failed to create CORS.");

    // =============================================================================
    // КОНФИГУРАЦИЯ SWAGGER UI
    // =============================================================================

    // Swagger UI — веб-интерфейс для просмотра и тестирования API.
    // openapi.json — автогенерируемая OpenAPI-спецификация всех эндпоинтов.
    let swagger = revolt_rocket_okapi::swagger_ui::make_swagger_ui(
        &revolt_rocket_okapi::swagger_ui::SwaggerUIConfig {
            url: "/openapi.json".to_owned(),
            ..Default::default()
        },
    )
    .into();

    // Отдельный Swagger для устаревшей версии API 0.8 (обратная совместимость)
    let swagger_0_8 = revolt_rocket_okapi::swagger_ui::make_swagger_ui(
        &revolt_rocket_okapi::swagger_ui::SwaggerUIConfig {
            url: "/0.8/openapi.json".to_owned(),
            ..Default::default()
        },
    )
    .into();

    let swagger_0_8 = revolt_rocket_okapi::swagger_ui::make_swagger_ui(
        &revolt_rocket_okapi::swagger_ui::SwaggerUIConfig {
            url: "/0.8/openapi.json".to_owned(),
            ..Default::default()
        },
    )
    .into();

    // =============================================================================
    // ГОЛОСОВЫЕ ЗВОНКИ И RABBITMQ
    // =============================================================================

    // VoiceClient управляет подключениями к LiveKit-нодам для голосовых звонков.
    // `config.api.livekit.nodes.clone()` — список LiveKit-серверов из конфига.
    let voice_client = VoiceClient::new(config.api.livekit.nodes.clone());

    // Подключаемся к RabbitMQ — брокеру сообщений для push-уведомлений.
    // RabbitMQ используется для асинхронной доставки: delta публикует задание,
    // pushd-демон подбирает его и отправляет уведомление на устройство.
    let connection = Connection::open(&OpenConnectionArguments::new(
        &config.rabbit.host,
        config.rabbit.port,
        &config.rabbit.username,
        &config.rabbit.password,
    ))
    .await
    .expect("Failed to connect to RabbitMQ");

    // Открываем канал внутри соединения (AMQP-канал — логическое разделение одного TCP-соединения)
    let channel = connection
        .open_channel(None)
        .await
        .expect("Failed to open RabbitMQ channel");

    // Объявляем exchange (точку маршрутизации) в RabbitMQ.
    // "direct" — маршрутизация по точному routing key.
    // `durable(true)` — exchange переживёт перезапуск RabbitMQ.
    channel
        .exchange_declare(
            ExchangeDeclareArguments::new(&config.pushd.exchange, "direct")
                .durable(true)
                .finish(),
        )
        .await
        .expect("Failed to declare exchange");

    // Создаём AMQP-обёртку для удобного использования в handlers
    let amqp = AMQP::new(connection, channel);

    // =============================================================================
    // ФОНОВЫЕ ЗАДАЧИ
    // =============================================================================

    // Запускаем воркеры для фоновых операций:
    // - Обработка embed'ов (парсинг URL для превью)
    // - Подтверждение прочтения сообщений
    // - Другие периодические задачи
    revolt_database::tasks::start_workers(db.clone(), amqp.clone());

    // =============================================================================
    // СБОРКА ROCKET-ПРИЛОЖЕНИЯ
    // =============================================================================

    // `rocket::build()` — создаёт "строитель" (builder) Rocket-приложения.
    let rocket = rocket::build();

    // Prometheus — система мониторинга метрик. PrometheusMetrics — middleware
    // который собирает статистику по каждому HTTP-запросу (статусы, латентность).
    let prometheus = PrometheusMetrics::new();

    // RatelimitStorage хранит информацию о rate limits для каждого клиента.
    // DeltaRatelimits определяет правила ограничений для конкретных эндпоинтов.
    let ratelimits = ratelimiter::RatelimitStorage::new(util::ratelimits::DeltaRatelimits);

    // Цепочка `.mount()` и `.manage()` — конфигурация Rocket:
    //
    // `.manage(value)` — регистрирует значение как глобальное состояние.
    //   Handlers могут получить его через `&State<T>` параметр.
    //   Это паттерн "Dependency Injection" в Rocket.
    //
    // `.mount(prefix, routes)` — регистрирует маршруты под заданным префиксом.
    //
    // `.attach(fairing)` — добавляет middleware (fairing):
    //   выполняется до/после каждого запроса (логирование, CORS, rate limits).
    routes::mount(config, rocket)
        .attach(prometheus.clone())
        .mount("/metrics", prometheus)                    // Эндпоинт /metrics для Prometheus scraping
        .mount("/", rocket_cors::catch_all_options_routes()) // CORS preflight (OPTIONS запросы)
        .mount("/", ratelimiter::routes())               // Rate limit эндпоинты
        .mount("/swagger/", swagger)                     // Swagger UI на /swagger/
        .mount("/0.8/swagger/", swagger_0_8)             // Swagger UI для v0.8 API
        .manage(authifier)                               // Система аутентификации
        .manage(db)                                      // Соединение с базой данных
        .manage(amqp)                                    // RabbitMQ для push-уведомлений
        .manage(cors.clone())                            // CORS-настройки
        .manage(voice_client)                            // LiveKit клиент
        .manage(ratelimits)                              // Rate limit хранилище
        .attach(ratelimiter::RatelimitFairing)           // Rate limiting middleware
        .attach(cors)                                    // CORS middleware
        .configure(rocket::Config {
            // Максимальный размер тела запроса: 5 MB (для загрузки контента)
            limits: rocket::data::Limits::default().limit("string", 5.megabytes()),
            // Слушаем на всех интерфейсах (0.0.0.0) — нужно для Docker/production
            address: Ipv4Addr::new(0, 0, 0, 0).into(),
            // Порт REST API сервера
            port: 14702,
            // `..Default::default()` — остальные настройки берём по умолчанию
            ..Default::default()
        })
}

// =============================================================================
// ТОЧКА ВХОДА ПРИЛОЖЕНИЯ
// =============================================================================

// `#[launch]` — макрос Rocket, генерирующий функцию `main()` которая:
// 1. Создаёт async runtime (tokio)
// 2. Вызывает нашу функцию `rocket()` для получения настроенного сервера
// 3. Запускает сервер и блокирует поток до завершения
// Возвращаемый тип `_` — Rust автоматически выводит тип из контекста.
#[launch]
async fn rocket() -> _ {
    // Инициализируем логирование и загружаем конфигурацию для API-сервиса.
    // `configure!(api)` — макрос revolt_config, настраивающий логгер и Sentry.
    revolt_config::configure!(api);

    // Запускаем веб-сервер
    web().await
}

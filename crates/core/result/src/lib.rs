// =============================================================================
// ФАЙЛ: crates/core/result/src/lib.rs
// НАЗНАЧЕНИЕ: Общие типы ошибок и результатов для всего бэкенда Stoat.
//   Этот крейт — фундамент системы обработки ошибок: все сервисы используют
//   типы отсюда вместо стандартных `std::result::Result` и `Box<dyn Error>`.
// =============================================================================

// Импортируем Location из стандартной библиотеки — он позволяет получить
// информацию о местоположении в исходном коде (файл, строка, столбец)
// прямо во время выполнения. Используется для отладки ошибок.
use std::panic::Location;
// Display — трейт из стандартной библиотеки для форматирования значений
// для вывода пользователю (метод fmt() вызывается при `println!("{}", value)`).
use std::fmt::Display;

// `#[cfg(feature = "serde")]` — условная компиляция: этот блок компилируется
// только если в Cargo.toml указана feature "serde". Это позволяет делать
// сериализацию опциональной — крейт можно использовать без зависимости от serde.
#[cfg(feature = "serde")]
#[macro_use]
extern crate serde;

// JsonSchema нужен для автоматической генерации JSON Schema — описания формата
// данных, используемого инструментами документации API (например, Swagger).
#[cfg(feature = "schemas")]
#[macro_use]
extern crate schemars;

// ToSchema из крейта utoipa используется для генерации OpenAPI-спецификации.
// OpenAPI — стандарт описания REST API, по которому генерируется документация.
#[cfg(feature = "utoipa")]
#[macro_use]
extern crate utoipa;

// Адаптер ошибок для фреймворка Rocket (REST API сервер delta).
// Этот модуль содержит реализацию, которая преобразует наш Error в HTTP-ответы.
#[cfg(feature = "rocket")]
pub mod rocket;

// Адаптер ошибок для фреймворка Axum (используется в сервисах autumn, january).
#[cfg(feature = "axum")]
pub mod axum;

// Адаптер для okapi — библиотеки генерации OpenAPI-документации для Rocket.
#[cfg(feature = "okapi")]
pub mod okapi;

// =============================================================================
// ПСЕВДОНИМ ТИПА РЕЗУЛЬТАТА
// =============================================================================

// `type Result<T, E = Error>` — псевдоним типа (type alias).
// Вместо того чтобы писать `std::result::Result<T, revolt_result::Error>`
// в каждой функции, мы определяем короткий вариант `Result<T>`.
// Параметр `E = Error` задаёт тип ошибки по умолчанию — наш собственный Error.
// Это очень распространённый паттерн в Rust-крейтах (например, std::io::Result).
/// Тип результата с нашим собственным типом ошибки по умолчанию.
pub type Result<T, E = Error> = std::result::Result<T, E>;

// =============================================================================
// СТРУКТУРА ОШИБКИ
// =============================================================================

// derive-атрибуты генерируют код автоматически:
// - Serialize, Deserialize — для преобразования в/из JSON (через serde)
// - JsonSchema — для генерации JSON Schema (документация API)
// - ToSchema — для генерации OpenAPI-описания
// - Debug — позволяет выводить значение через `{:?}` (для отладки)
// - Clone — позволяет создавать копию значения методом `.clone()`
/// Информация об ошибке: тип + место возникновения в коде.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[cfg_attr(feature = "utoipa", derive(ToSchema))]
#[derive(Debug, Clone)]
pub struct Error {
    // `#[serde(flatten)]` — директива serde, которая "разворачивает" вложенную
    // структуру: вместо `{"error_type": {"type": "NotFound"}}` получим `{"type": "NotFound"}`.
    // Это упрощает JSON-формат ошибок, который видит клиент.
    /// Тип ошибки с дополнительными данными
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub error_type: ErrorType,

    /// Место возникновения ошибки в коде (формат: "файл:строка:столбец")
    pub location: String,
}

// `impl Display for Error` — реализация трейта Display для нашего типа Error.
// Это стандартный способ задать "человекочитаемое" строковое представление.
// Трейт требует реализации метода `fmt()`.
impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `write!` записывает отформатированную строку в formatter.
        // `{:?}` — использует Debug-форматирование для error_type (выводит имя варианта enum).
        write!(f, "{:?} occurred in {}", self.error_type, self.location)
    }
}

// Реализация маркерного трейта `std::error::Error` — пустая, но необходима,
// чтобы наш Error был совместим со стандартной экосистемой обработки ошибок Rust
// (например, для использования с оператором `?` и библиотекой `anyhow`).
impl std::error::Error for Error {}

// =============================================================================
// ПЕРЕЧИСЛЕНИЕ ТИПОВ ОШИБОК
// =============================================================================

// `#[serde(tag = "type")]` — директива serde для tagged union.
// При сериализации в JSON каждый вариант enum получит поле "type" с именем варианта.
// Например: {"type": "UnknownUser"} или {"type": "TooManyAttachments", "max": 10}.
// Это стандартный паттерн для REST API — клиент может программно проверить тип ошибки.
/// Все возможные типы ошибок в системе.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type"))]
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[cfg_attr(feature = "utoipa", derive(ToSchema))]
#[derive(Debug, Clone)]
pub enum ErrorType {
    /// Ошибка не классифицирована — используется как временная заглушка при разработке.
    LabelMe,

    // ? Ошибки онбординга (первичной настройки аккаунта)
    AlreadyOnboarded,

    // ? Ошибки, связанные с пользователями
    UsernameTaken,
    InvalidUsername,
    DiscriminatorChangeRatelimited,
    UnknownUser,
    AlreadyFriends,
    AlreadySentRequest,
    Blocked,
    BlockedByOther,
    NotFriends,
    // Варианты enum могут содержать данные — здесь `max: usize` сообщает
    // клиенту конкретный лимит, при котором произошла ошибка.
    TooManyPendingFriendRequests {
        max: usize,
    },

    // ? Ошибки, связанные с каналами
    UnknownChannel,
    UnknownAttachment,
    UnknownMessage,
    CannotEditMessage,
    CannotJoinCall,
    TooManyAttachments {
        max: usize,
    },
    TooManyEmbeds {
        max: usize,
    },
    TooManyReplies {
        max: usize,
    },
    TooManyChannels {
        max: usize,
    },
    EmptyMessage,
    PayloadTooLarge,
    CannotRemoveYourself,
    GroupTooLarge {
        max: usize,
    },
    AlreadyInGroup,
    NotInGroup,
    AlreadyPinned,
    NotPinned,

    // ? Ошибки, связанные с серверами (сообществами)
    UnknownServer,
    InvalidRole,
    Banned,
    TooManyServers {
        max: usize,
    },
    TooManyEmoji {
        max: usize,
    },
    TooManyRoles {
        max: usize,
    },
    AlreadyInServer,
    CannotTimeoutYourself,

    // ? Ошибки, связанные с ботами
    ReachedMaximumBots,
    IsBot,
    IsNotBot,
    BotIsPrivate,

    // ? Ошибки безопасности пользователей
    CannotReportYourself,

    // ? Ошибки прав доступа
    // Содержат имя конкретного права, которого не хватает — удобно для отладки
    MissingPermission {
        permission: String,
    },
    MissingUserPermission {
        permission: String,
    },
    NotElevated,
    NotPrivileged,
    CannotGiveMissingPermissions,
    NotOwner,
    IsElevated,

    // ? Общие/системные ошибки
    // DatabaseError содержит операцию и коллекцию — показывает, где именно
    // в базе данных произошла ошибка. Используется в production-сборках
    // вместо полного раскрытия деталей (предотвращает утечку внутренней структуры).
    DatabaseError {
        operation: String,
        collection: String,
    },
    InternalError,
    InvalidOperation,
    InvalidCredentials,
    InvalidProperty,
    InvalidSession,
    InvalidFlagValue,
    NotAuthenticated,
    DuplicateNonce,
    NotFound,
    NoEffect,
    FailedValidation {
        error: String,
    },

    // ? Ошибки голосовых звонков (LiveKit)
    LiveKitUnavailable,
    NotAVoiceChannel,
    AlreadyConnected,
    NotConnected,
    UnknownNode,

    // ? Ошибки микросервисов (файловый сервис autumn, прокси january)
    ProxyError,
    FileTooSmall,
    FileTooLarge {
        max: usize,
    },
    FileTypeNotAllowed,
    ImageProcessingFailed,
    NoEmbedData,

    // ? Устаревшие ошибки (legacy, для обратной совместимости)
    VosoUnavailable,

    // ? Ошибка отключённой функциональности (feature flags из конфига)
    FeatureDisabled {
        feature: String,
    },
}

// =============================================================================
// МАКРОСЫ ДЛЯ СОЗДАНИЯ ОШИБОК
// =============================================================================

// `#[macro_export]` делает макрос доступным для пользователей крейта —
// они смогут использовать `create_error!` без явного импорта.
#[macro_export]
macro_rules! create_error {
    // Паттерн: имя варианта ErrorType и опциональные данные через `$( $tt:tt )?`.
    // `$tt:tt` — "token tree", самый общий паттерн, принимает любые токены Rust.
    ( $error: ident $( $tt:tt )? ) => {
        $crate::Error {
            error_type: $crate::ErrorType::$error $( $tt )?,
            // `file!()`, `line!()`, `column!()` — встроенные макросы Rust,
            // которые подставляют имя файла, номер строки и столбца
            // прямо в момент компиляции. Это бесплатно в runtime!
            location: format!("{}:{}:{}", file!(), line!(), column!()),
        }
    };
}

// Специализированный макрос для ошибок базы данных.
// Удобная обёртка над create_error! — не нужно писать имена полей вручную.
#[macro_export]
macro_rules! create_database_error {
    ( $operation: expr, $collection: expr ) => {
        $crate::create_error!(DatabaseError {
            operation: $operation.to_string(),
            collection: $collection.to_string()
        })
    };
}

// Макрос `query!` используется в слоях базы данных.
// Существует в двух вариантах: для debug и release сборок.
//
// DEBUG-вариант (разработка): вызывает `unwrap()`, что вызовет панику
// при ошибке. Это позволяет быстро обнаружить проблему во время разработки.
#[macro_export]
#[cfg(debug_assertions)]
macro_rules! query {
    ( $self: ident, $type: ident, $collection: expr, $($rest:expr),+ ) => {
        Ok($self.$type($collection, $($rest),+).await.unwrap())
    };
}

// RELEASE-вариант (продакшн): преобразует ошибку в DatabaseError вместо паники.
// Это безопаснее — сервер не упадёт, а вернёт HTTP 500 клиенту.
// `stringify!($type)` — превращает идентификатор в строку во время компиляции.
#[macro_export]
#[cfg(not(debug_assertions))]
macro_rules! query {
    ( $self: ident, $type: ident, $collection: expr, $($rest:expr),+ ) => {
        $self.$type($collection, $($rest),+).await
            .map_err(|_| create_database_error!(stringify!($type), $collection))
    };
}

// =============================================================================
// ТРЕЙТ ToRevoltError — КОНВЕРТАЦИЯ СТОРОННИХ ОШИБОК
// =============================================================================

// Трейт с generic-параметром `T` — позволяет конвертировать любой Result<T, E>
// или Option<T> в наш стандартный Result<T, Error>.
// Это особенно полезно при работе с внешними библиотеками, у которых свои типы ошибок.
pub trait ToRevoltError<T> {
    // `#[track_caller]` — атрибут, который сохраняет место вызова функции
    // для последующего получения через `Location::caller()`.
    // Без него Location::caller() возвращал бы место внутри этой функции, а не снаружи.
    #[track_caller]
    fn to_internal_error(self) -> Result<T, Error>;
}

// Реализация трейта для Result<T, E> где E реализует Debug + Error.
// `impl<T, E: std::fmt::Debug + std::error::Error>` — обобщённая реализация
// для любого типа ошибки, который поддерживает отладочный вывод.
impl<T, E: std::fmt::Debug + std::error::Error> ToRevoltError<T> for Result<T, E> {
    #[track_caller]
    fn to_internal_error(self) -> Result<T, Error> {
        // `Location::caller()` — получаем место вызова этого метода
        // (не место определения, а место использования в коде).
        let loc = Location::caller();

        self
            // `.map_err()` — преобразует ошибку: если Result::Err(e), применяем замыкание.
            // Если Result::Ok(v), значение остаётся без изменений.
            .map_err(|e| {
                // Логируем исходную ошибку для внутреннего мониторинга
                log::error!("{e:?}");
                // Если включена feature "sentry", отправляем ошибку в Sentry
                // (сервис мониторинга ошибок в production-среде)
                #[cfg(feature = "sentry")]
                sentry::capture_error(&e);

                // Клиенту возвращаем общую InternalError — не раскрываем детали реализации
                Error {
                    error_type: ErrorType::InternalError,
                    location: format!("{}:{}:{}", loc.file(), loc.line(), loc.column())
                }
            })
    }
}

// Реализация для Option<T> — конвертирует None в InternalError.
// Паттерн: когда мы ожидаем значение, но его нет, это системная ошибка.
impl<T> ToRevoltError<T> for Option<T> {
    #[track_caller]
    fn to_internal_error(self) -> Result<T, Error> {
        let loc = Location::caller();

        // `.ok_or_else()` — конвертирует Option в Result:
        // Some(v) → Ok(v), None → Err(замыкание())
        self.ok_or_else(|| {
            Error {
                error_type: ErrorType::InternalError,
                location: format!("{}:{}:{}", loc.file(), loc.line(), loc.column())
            }
        })
    }
}

// =============================================================================
// ТЕСТЫ
// =============================================================================

// Этот модуль компилируется только при запуске тестов (`cargo test`)
#[cfg(test)]
mod tests {
    use crate::ErrorType;

    // `#[test]` — атрибут, помечающий функцию как тестовую.
    // cargo test автоматически находит и запускает такие функции.
    #[test]
    fn use_macro_to_construct_error() {
        // Проверяем что макрос создаёт ошибку нужного типа
        let error = create_error!(LabelMe);
        // `assert!(matches!(...))` — утверждение с паттерн-матчингом.
        // Тест упадёт, если ошибка имеет неверный тип.
        assert!(matches!(error.error_type, ErrorType::LabelMe));
    }

    #[test]
    fn use_macro_to_construct_complex_error() {
        let error = create_error!(LabelMe);
        assert!(matches!(error.error_type, ErrorType::LabelMe));
    }
}

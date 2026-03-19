// =============================================================================
// ФАЙЛ: crates/delta/src/routes/users/fetch_user.rs
// НАЗНАЧЕНИЕ: HTTP GET-обработчик для получения информации о пользователе.
//   Реализует эндпоинт: GET /users/<id>
//
// КЛЮЧЕВЫЕ КОНЦЕПТЫ:
// - Rocket route macro (#[get]) — декларативное объявление HTTP-маршрута
// - #[openapi] — генерация OpenAPI/Swagger документации
// - State<T> — глобальное состояние приложения (инъекция зависимостей)
// - Request Guards — автоматическое извлечение данных из запроса (User, Reference)
// - Оператор ? — распространение ошибок
// =============================================================================

// `use` — импорт типов из других крейтов.
// В Rocket каждый параметр функции-обработчика может быть "request guard" —
// Rocket автоматически извлекает его из HTTP-запроса или возвращает ошибку.
use revolt_database::{
    util::{
        // `DatabasePermissionQuery` — реализует трейт PermissionQuery для нашей БД.
        // Позволяет проверять права доступа в контексте конкретного пользователя.
        permissions::DatabasePermissionQuery,
        // `Reference<'_>` — request guard: извлекает идентификатор из URL-параметра.
        // Поддерживает как ULID-строки, так и специальные алиасы (например "@me").
        // `'_` — анонимный lifetime: Reference заимствует данные из запроса.
        reference::Reference,
    },
    Database, User,
};
// `v0` — API-модели версии 0 (данные, отправляемые клиентам).
// Они могут отличаться от внутренних моделей БД — это намеренная развязка.
use revolt_models::v0;

// Функции и типы для проверки прав доступа между пользователями.
use revolt_permissions::{calculate_user_permissions, UserPermission};
// Наш стандартный тип Result (см. crates/core/result/).
use revolt_result::Result;
// `Json<T>` — сериализует T в JSON и устанавливает Content-Type: application/json.
// `State<T>` — request guard для получения глобального состояния (Database, AMQP и т.д.).
use rocket::{serde::json::Json, State};

// =============================================================================
// HTTP-ОБРАБОТЧИК
// =============================================================================

/// Получение информации о пользователе.
///
/// Возвращает публичный профиль пользователя с учётом прав доступа запрашивающего.
// `#[openapi(tag = "User Information")]` — включает этот эндпоинт в OpenAPI-документацию
// под тегом "User Information". Swagger UI группирует эндпоинты по тегам.
#[openapi(tag = "User Information")]
// `#[get("/<target>")]` — Rocket-макрос, объявляющий HTTP GET-маршрут.
// `<target>` — динамический сегмент URL: /users/01HX... или /users/@me.
// Rocket автоматически передаёт его в параметр `target: Reference<'_>`.
#[get("/<target>")]
// `pub async fn fetch(...)` — публичная асинхронная функция.
// Параметры функции — это "request guards". Rocket автоматически:
//   1. Извлекает `db` из глобального состояния (manage(db) в main.rs)
//   2. Аутентифицирует пользователя и создаёт `user: User` (или возвращает 401)
//   3. Парсит URL-параметр в `target: Reference<'_>`
// `-> Result<Json<v0::User>>` — возвращает либо JSON-пользователя, либо ошибку.
// Rocket конвертирует ошибки в соответствующие HTTP-статусы (404, 403 и т.д.).
pub async fn fetch(db: &State<Database>, user: User, target: Reference<'_>) -> Result<Json<v0::User>> {
    // Оптимизация: если пользователь запрашивает сам себя — возвращаем сразу.
    // Нет нужды в проверке прав или запросе к БД.
    // `user.id == target.id` — сравнение строк (String implements PartialEq).
    if user.id == target.id {
        // `into_self(false)` — конвертирует внутреннюю модель User в API-модель v0::User.
        // Параметр `false` — не раскрывать отношения (relationship data).
        return Ok(Json(user.into_self(false).await));
    }

    // Загружаем профиль целевого пользователя из БД.
    // `?` — оператор распространения ошибок: если `as_user` вернул Err,
    // функция немедленно возвращает эту ошибку. Аналог `try-catch` без бойлерплейта.
    let target = target.as_user(db).await?;

    // Проверяем, имеет ли текущий пользователь право видеть целевого.
    // `DatabasePermissionQuery::new(db, &user).user(&target)` — builder-паттерн:
    // создаём контекст запроса прав с указанием обоих участников.
    let mut query = DatabasePermissionQuery::new(db, &user).user(&target);

    // `calculate_user_permissions` — вычисляет итоговые права из всех факторов:
    // блокировки, настройки приватности, общие серверы и т.д.
    // `.throw_if_lacking_user_permission(UserPermission::Access)?` —
    // если права недостаточны, выбрасывает MissingUserPermission ошибку (→ HTTP 403).
    // Ещё один `?` для распространения этой ошибки.
    calculate_user_permissions(&mut query)
        .await
        .throw_if_lacking_user_permission(UserPermission::Access)?;

    // Конвертируем внутреннюю модель в API-модель и возвращаем как JSON.
    // `target.into(db, &user)` — метод конвертации учитывает контекст `user`,
    // чтобы правильно заполнить поля (например, relationship, mutual servers).
    Ok(Json(target.into(db, &user).await))
}

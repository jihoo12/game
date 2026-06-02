use axum::{
    response::{Html, Json, IntoResponse},
    routing::{get, post},
    Router,
    http::StatusCode,
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::fs;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Card {
    title: String,
    rarity: String,
    bonus_multiplier: f32,
}

// [변경] Higher or Lower에 맞게 게임 세션 상태 정의
struct GameState {
    left_title: String,
    right_title: String,
    left_len: usize,
    right_len: usize,
    points: i32,
    inventory: Vec<Card>,
    active_card: Option<Card>,
}

impl Default for GameState {
    fn default() -> Self {
        Self {
            left_title: String::new(),
            right_title: String::new(),
            left_len: 0,
            right_len: 0,
            points: 100, // 가챠 1회분 무료 지급
            inventory: Vec::new(),
            active_card: None,
        }
    }
}

#[derive(Deserialize, Serialize, Clone)]
struct WikiResponse {
    title: String,
    extract: Option<String>,
}

// 프론트엔드로 전달할 신규 매치 정보
#[derive(Serialize)]
struct NewGameResponse {
    left_title: String,
    left_extract: String,
    left_len: usize,
    right_title: String,
    right_extract: String,
    points: i32,
    active_card: Option<Card>,
}

// 유저의 선택 데이터 요청받기
#[derive(Deserialize)]
struct ChoiceRequest {
    user_choice: String, // "Higher" 또는 "Lower"
}

// 결과 반환
#[derive(Serialize)]
struct ResultResponse {
    is_correct: bool,
    right_len: usize,
    points: i32,
    earned_points: i32,
}

#[derive(Serialize)]
struct GachaResponse {
    success: bool,
    message: String,
    card: Option<Card>,
    points: i32,
    inventory: Vec<Card>,
}

#[derive(Deserialize)]
struct EquipRequest {
    index: usize,
}

type SharedState = Arc<Mutex<GameState>>;

#[tokio::main]
async fn main() {
    let shared_state = Arc::new(Mutex::new(GameState::default()));

    let app = Router::new()
        .route("/", get(home_handler))
        .route("/api/new-game", get(get_new_match))
        .route("/api/submit", post(check_choice))
        .route("/api/gacha", post(perform_gacha))
        .route("/api/equip", post(equip_card))
        .with_state(shared_state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    println!("🚀 위키 Higher-or-Lower 게임 서버가 http://127.0.0.1:3000 에서 오픈되었습니다!");
    axum::serve(listener, app).await.unwrap();
}

async fn home_handler() -> impl IntoResponse {
    match fs::read_to_string("templates/index.html") {
        Ok(html_content) => Html(html_content).into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "오류: templates/index.html 파일을 찾을 수 없습니다."
        ).into_response(),
    }
}

// 위키백과 랜덤 요약본을 가져오는 공통 헬퍼 함수
async fn fetch_random_wiki() -> Option<WikiResponse> {
    let client = reqwest::Client::new();
    let url = "https://ko.wikipedia.org/api/rest_v1/page/random/summary";
    let response = client
        .get(url)
        .header("User-Agent", "WikiHigherLowerGame/1.0 (contact@example.com)")
        .send()
        .await;

    if let Ok(res) = response {
        if let Ok(wiki_data) = res.json::<WikiResponse>().await {
            return Some(wiki_data);
        }
    }
    None
}

// [변경] 두 개의 랜덤 위키 문서를 가져와 매칭시키는 핸들러
async fn get_new_match(
    axum::extract::State(state): axum::extract::State<SharedState>,
) -> Json<NewGameResponse> {
    // 동기식 처리를 피해 비동기로 2개의 독립적인 페이지 요청을 보냅니다.
    let left_page = fetch_random_wiki().await;
    let right_page = fetch_random_wiki().await;

    let mut lock = state.lock().unwrap();

    if let (Some(left), Some(right)) = (left_page, right_page) {
        lock.left_title = left.title;
        let left_extract = left.extract.unwrap_or_else(|| "본문 없음".to_string());
        // 글자 수 계산 (.chars().count() 로 해야 한글 자수 매칭이 정확합니다)
        lock.left_len = left_extract.chars().count();

        lock.right_title = right.title;
        let right_extract = right.extract.unwrap_or_else(|| "본문 없음".to_string());
        lock.right_len = right_extract.chars().count();

        return Json(NewGameResponse {
            left_title: lock.left_title.clone(),
            left_extract,
            left_len: lock.left_len,
            right_title: lock.right_title.clone(),
            right_extract,
            points: lock.points,
            active_card: lock.active_card.clone(),
        });
    }

    // 예외 상황 처리
    Json(NewGameResponse {
        left_title: "에러 발생".to_string(),
        left_extract: "위키백과 데이터를 가져오지 못했습니다. 다시 시도해 주세요.".to_string(),
        left_len: 0,
        right_title: "에러 발생".to_string(),
        right_extract: "재시도가 필요합니다.".to_string(),
        points: lock.points,
        active_card: lock.active_card.clone(),
    })
}

// [변경] 유저의 선택(Higher/Lower)을 판정하는 핸들러
async fn check_choice(
    axum::extract::State(state): axum::extract::State<SharedState>,
    Json(payload): Json<ChoiceRequest>,
) -> Json<ResultResponse> {
    let mut lock = state.lock().unwrap();

    // 판정 로직: 오른쪽 글자 수가 왼쪽보다 크거나 같은 경우 Higher가 정답
    let actual_result = if lock.right_len >= lock.left_len { "Higher" } else { "Lower" };
    let is_correct = payload.user_choice == actual_result;
    
    let mut earned_points = 0;

    if is_correct {
        let multiplier = match &lock.active_card {
            Some(card) => card.bonus_multiplier,
            None => 1.0,
        };
        earned_points = (20.0 * multiplier) as i32;
        lock.points += earned_points;
    } else {
        lock.points = std::cmp::max(0, lock.points - 5);
    }

    Json(ResultResponse {
        is_correct,
        right_len: lock.right_len,
        points: lock.points,
        earned_points,
    })
}

// 뽑기(가챠) 핸들러
async fn perform_gacha(
    axum::extract::State(state): axum::extract::State<SharedState>,
) -> Json<GachaResponse> {
    let mut lock = state.lock().unwrap();

    if lock.points < 100 {
        return Json(GachaResponse {
            success: false,
            message: "포인트가 부족합니다! 매치를 더 진행해 보세요.".to_string(),
            card: None,
            points: lock.points,
            inventory: lock.inventory.clone(),
        });
    }

    lock.points -= 100;

    let mut rng = rand::thread_rng();
    let rand_val: f32 = rng.gen_range(0.0..100.0);

    let (rarity, multiplier) = if rand_val < 5.0 {
        ("SSR", 3.0)
    } else if rand_val < 25.0 {
        ("SR", 1.8)
    } else {
        ("R", 1.2)
    };

    // 현재 게임에 등장한 두 문서 중 하나를 랜덤으로 카드 이름으로 채택!
    let card_name = if !lock.left_title.is_empty() && rng.gen_bool(0.5) {
        lock.left_title.clone()
    } else if !lock.right_title.is_empty() {
        lock.right_title.clone()
    } else {
        "이름없는 위키 문서".to_string()
    };

    let new_card = Card {
        title: card_name,
        rarity: rarity.to_string(),
        bonus_multiplier: multiplier,
    };

    lock.inventory.push(new_card.clone());

    Json(GachaResponse {
        success: true,
        message: "가챠 성공!".to_string(),
        card: Some(new_card),
        points: lock.points,
        inventory: lock.inventory.clone(),
    })
}

// 카드 장착 핸들러
async fn equip_card(
    axum::extract::State(state): axum::extract::State<SharedState>,
    Json(payload): Json<EquipRequest>,
) -> Json<String> {
    let mut lock = state.lock().unwrap();
    if let Some(card) = lock.inventory.get(payload.index) {
        lock.active_card = Some(card.clone());
        Json("장착 완료!".to_string())
    } else {
        Json("존재하지 않는 번호입니다.".to_string())
    }
}
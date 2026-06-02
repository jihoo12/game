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

// ─── Data Types ───────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
struct BattleCard {
    id: u64,
    title: String,
    extract: String,
    rarity: String,      // "N", "R", "SR", "SSR"
    element: String,     // "Fire", "Water", "Wind", "Earth", "Dark", "Light"
    hp: i32,
    atk: i32,
    def: i32,
    spd: i32,
    skill_name: String,
    skill_desc: String,
    skill_multiplier: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct BattleState {
    player_card: Option<BattleCard>,
    enemy_card: Option<BattleCard>,
    player_hp: i32,
    enemy_hp: i32,
    player_max_hp: i32,
    enemy_max_hp: i32,
    turn: u32,
    log: Vec<String>,
    battle_over: bool,
    player_won: Option<bool>,
}

impl Default for BattleState {
    fn default() -> Self {
        Self {
            player_card: None,
            enemy_card: None,
            player_hp: 0,
            enemy_hp: 0,
            player_max_hp: 0,
            enemy_max_hp: 0,
            turn: 1,
            log: Vec::new(),
            battle_over: false,
            player_won: None,
        }
    }
}

struct GameState {
    gems: i32,
    pity_count: i32,       // pulls since last SR+
    pity_sr_count: i32,    // pulls since last SSR
    deck: Vec<BattleCard>, // player's card collection
    next_id: u64,
    battle: BattleState,
    skill_used: bool,
}

impl Default for GameState {
    fn default() -> Self {
        Self {
            gems: 300, // start with 3 free pulls
            pity_count: 0,
            pity_sr_count: 0,
            deck: Vec::new(),
            next_id: 1,
            battle: BattleState::default(),
            skill_used: false,
        }
    }
}

type SharedState = Arc<Mutex<GameState>>;

// ─── API Request/Response Types ───────────────────────────────────────────────

#[derive(Deserialize, Clone)]
struct WikiResponse {
    title: String,
    extract: Option<String>,
}

#[derive(Serialize)]
struct StatusResponse {
    gems: i32,
    pity_count: i32,
    pity_sr_count: i32,
    deck: Vec<BattleCard>,
}

#[derive(Serialize)]
struct GachaResult {
    cards: Vec<BattleCard>,
    gems: i32,
    pity_count: i32,
    pity_sr_count: i32,
}

#[derive(Deserialize)]
struct BattleStartRequest {
    player_card_id: u64,
}

#[derive(Serialize)]
struct BattleStartResponse {
    battle: BattleState,
    success: bool,
    message: String,
}

#[derive(Deserialize)]
struct BattleActionRequest {
    action: String, // "attack", "skill", "defend"
}

#[derive(Serialize)]
struct BattleActionResponse {
    battle: BattleState,
    gems_earned: i32,
    gems: i32,
}

// ─── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let shared_state = Arc::new(Mutex::new(GameState::default()));

    let app = Router::new()
        .route("/", get(home_handler))
        .route("/api/status", get(get_status))
        .route("/api/gacha", post(perform_gacha))
        .route("/api/gacha/ten", post(perform_ten_pull))
        .route("/api/battle/start", post(start_battle))
        .route("/api/battle/action", post(battle_action))
        .with_state(shared_state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    println!("🃏 WikiBattle server running at http://127.0.0.1:3000");
    axum::serve(listener, app).await.unwrap();
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

async fn home_handler() -> impl IntoResponse {
    match fs::read_to_string("templates/index.html") {
        Ok(html) => Html(html).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "templates/index.html not found").into_response(),
    }
}

async fn get_status(
    axum::extract::State(state): axum::extract::State<SharedState>,
) -> Json<StatusResponse> {
    let lock = state.lock().unwrap();
    Json(StatusResponse {
        gems: lock.gems,
        pity_count: lock.pity_count,
        pity_sr_count: lock.pity_sr_count,
        deck: lock.deck.clone(),
    })
}

// ─── Wiki Helper ──────────────────────────────────────────────────────────────

async fn fetch_random_wiki() -> Option<WikiResponse> {
    let client = reqwest::Client::new();
    let res = client
        .get("https://ko.wikipedia.org/api/rest_v1/page/random/summary")
        .header("User-Agent", "WikiBattleGame/2.0")
        .send()
        .await
        .ok()?;
    res.json::<WikiResponse>().await.ok()
}

fn derive_card(wiki: &WikiResponse, rarity: &str, id: u64) -> BattleCard {
    let extract = wiki.extract.clone().unwrap_or_default();
    let text_len = extract.chars().count() as i32;
    let title_len = wiki.title.chars().count() as i32;

    let elements = ["Fire", "Water", "Wind", "Earth", "Dark", "Light"];
    let element_idx = (text_len + title_len) as usize % elements.len();
    let element = elements[element_idx].to_string();

    // Stat scaling by rarity
    let scale: f32 = match rarity {
        "SSR" => 2.5,
        "SR"  => 1.7,
        "R"   => 1.2,
        _     => 1.0, // N
    };

    let base_hp  = 80 + (text_len / 10).min(120);
    let base_atk = 15 + (title_len * 3).min(60);
    let base_def = 10 + (text_len / 30).min(40);
    let base_spd = 10 + (title_len % 20);

    let hp  = ((base_hp  as f32) * scale) as i32;
    let atk = ((base_atk as f32) * scale) as i32;
    let def = ((base_def as f32) * scale) as i32;
    let spd = ((base_spd as f32) * scale) as i32;

    let (skill_name, skill_desc, skill_multiplier) = match element.as_str() {
        "Fire"  => ("화염 폭풍", "강렬한 불꽃으로 적을 태운다. ATK×2.2 피해", 2.2),
        "Water" => ("조류 파동", "물결로 적을 압도한다. ATK×2.0 + 방어 무시", 2.0),
        "Wind"  => ("질풍 난무", "빠른 바람으로 3번 공격. ATK×1.6×3", 1.6),
        "Earth" => ("대지 분쇄", "땅을 가르는 일격. ATK×2.5 피해", 2.5),
        "Dark"  => ("암흑 붕괴", "어둠의 힘으로 적 HP의 30% 추가 피해", 2.1),
        _       => ("성광 심판", "빛의 심판. ATK×2.3 + 자신 HP 15 회복", 2.3),
    };

    BattleCard {
        id,
        title: wiki.title.clone(),
        extract: extract.chars().take(120).collect::<String>() + "…",
        rarity: rarity.to_string(),
        element,
        hp,
        atk,
        def,
        spd,
        skill_name: skill_name.to_string(),
        skill_desc: skill_desc.to_string(),
        skill_multiplier,
    }
}

fn determine_rarity(pity: i32, pity_sr: i32, rng: &mut impl Rng) -> &'static str {
    // Pity: guaranteed SSR at 50 pulls, SR at 10 pulls
    if pity_sr >= 49 {
        return "SSR";
    }
    if pity >= 9 {
        return "SR";
    }
    let roll: f32 = rng.gen_range(0.0..100.0);
    if roll < 2.5 { "SSR" }
    else if roll < 15.0 { "SR" }
    else if roll < 50.0 { "R" }
    else { "N" }
}

// ─── Gacha ────────────────────────────────────────────────────────────────────

async fn perform_gacha(
    axum::extract::State(state): axum::extract::State<SharedState>,
) -> Json<serde_json::Value> {
    let cost = 100;
    {
        let lock = state.lock().unwrap();
        if lock.gems < cost {
            return Json(serde_json::json!({ "error": "젬이 부족합니다!", "gems": lock.gems }));
        }
    }

    let wiki = fetch_random_wiki().await;
    let wiki = match wiki {
        Some(w) => w,
        None => return Json(serde_json::json!({ "error": "위키 데이터 오류" })),
    };

    let mut lock = state.lock().unwrap();
    lock.gems -= cost;
    lock.pity_count += 1;
    lock.pity_sr_count += 1;

    let mut rng = rand::thread_rng();
    let rarity = determine_rarity(lock.pity_count, lock.pity_sr_count, &mut rng);

    // Reset pity counters
    if rarity == "SSR" {
        lock.pity_count = 0;
        lock.pity_sr_count = 0;
    } else if rarity == "SR" {
        lock.pity_count = 0;
    }

    let id = lock.next_id;
    lock.next_id += 1;
    let card = derive_card(&wiki, rarity, id);
    lock.deck.push(card.clone());

    Json(serde_json::json!({
        "cards": [card],
        "gems": lock.gems,
        "pity_count": lock.pity_count,
        "pity_sr_count": lock.pity_sr_count,
    }))
}

async fn perform_ten_pull(
    axum::extract::State(state): axum::extract::State<SharedState>,
) -> Json<serde_json::Value> {
    let cost = 900; // 10 pulls, 1 free
    {
        let lock = state.lock().unwrap();
        if lock.gems < cost {
            return Json(serde_json::json!({ "error": "젬이 부족합니다!", "gems": lock.gems }));
        }
    }

    // Fetch 10 wiki pages concurrently
    let futures: Vec<_> = (0..10).map(|_| fetch_random_wiki()).collect();
    let results = futures::future::join_all(futures).await;

    let mut lock = state.lock().unwrap();
    lock.gems -= cost;

    let mut rng = rand::thread_rng();
    let mut cards = Vec::new();

    for (i, wiki_opt) in results.into_iter().enumerate() {
        let wiki = match wiki_opt {
            Some(w) => w,
            None => continue,
        };

        lock.pity_count += 1;
        lock.pity_sr_count += 1;

        // Guarantee at least one SR in 10-pull
        let rarity = if i == 9 && cards.iter().all(|c: &BattleCard| c.rarity == "N" || c.rarity == "R") {
            // Force SR minimum on last card
            if lock.pity_sr_count >= 49 { "SSR" } else { "SR" }
        } else {
            determine_rarity(lock.pity_count, lock.pity_sr_count, &mut rng)
        };

        if rarity == "SSR" {
            lock.pity_count = 0;
            lock.pity_sr_count = 0;
        } else if rarity == "SR" {
            lock.pity_count = 0;
        }

        let id = lock.next_id;
        lock.next_id += 1;
        let card = derive_card(&wiki, rarity, id);
        lock.deck.push(card.clone());
        cards.push(card);
    }

    Json(serde_json::json!({
        "cards": cards,
        "gems": lock.gems,
        "pity_count": lock.pity_count,
        "pity_sr_count": lock.pity_sr_count,
    }))
}

// ─── Battle ───────────────────────────────────────────────────────────────────

async fn start_battle(
    axum::extract::State(state): axum::extract::State<SharedState>,
    Json(payload): Json<BattleStartRequest>,
) -> Json<BattleStartResponse> {
    let enemy_wiki = fetch_random_wiki().await;
    let enemy_wiki = match enemy_wiki {
        Some(w) => w,
        None => return Json(BattleStartResponse {
            battle: BattleState::default(),
            success: false,
            message: "위키 데이터를 가져오지 못했습니다.".to_string(),
        }),
    };

    let mut lock = state.lock().unwrap();
    lock.skill_used = false;

    let player_card = lock.deck.iter().find(|c| c.id == payload.player_card_id).cloned();
    let player_card = match player_card {
        Some(c) => c,
        None => return Json(BattleStartResponse {
            battle: BattleState::default(),
            success: false,
            message: "카드를 찾을 수 없습니다.".to_string(),
        }),
    };

    let mut rng = rand::thread_rng();
    let enemy_rarities = ["N", "R", "SR"];
    let enemy_rarity = enemy_rarities[rng.gen_range(0..enemy_rarities.len())];
    let id = lock.next_id;
    lock.next_id += 1;
    let enemy_card = derive_card(&enemy_wiki, enemy_rarity, id);

    let player_hp = player_card.hp;
    let enemy_hp = enemy_card.hp;

    let battle = BattleState {
        player_card: Some(player_card),
        enemy_card: Some(enemy_card.clone()),
        player_hp,
        enemy_hp,
        player_max_hp: player_hp,
        enemy_max_hp: enemy_hp,
        turn: 1,
        log: vec![format!("⚔️ {} 과(와) 전투 시작!", enemy_card.title)],
        battle_over: false,
        player_won: None,
    };

    lock.battle = battle.clone();

    Json(BattleStartResponse {
        battle,
        success: true,
        message: "전투 시작!".to_string(),
    })
}

async fn battle_action(
    axum::extract::State(state): axum::extract::State<SharedState>,
    Json(payload): Json<BattleActionRequest>,
) -> Json<BattleActionResponse> {
    let mut lock = state.lock().unwrap();
    let mut gems_earned = 0;

    if lock.battle.battle_over {
        return Json(BattleActionResponse {
            battle: lock.battle.clone(),
            gems_earned: 0,
            gems: lock.gems,
        });
    }

    let player = match &lock.battle.player_card {
        Some(c) => c.clone(),
        None => return Json(BattleActionResponse {
            battle: lock.battle.clone(),
            gems_earned: 0,
            gems: lock.gems,
        }),
    };
    let enemy = match &lock.battle.enemy_card {
        Some(c) => c.clone(),
        None => return Json(BattleActionResponse {
            battle: lock.battle.clone(),
            gems_earned: 0,
            gems: lock.gems,
        }),
    };

    let mut rng = rand::thread_rng();
    let mut log = lock.battle.log.clone();
    let mut player_hp = lock.battle.player_hp;
    let mut enemy_hp = lock.battle.enemy_hp;
    let turn = lock.battle.turn;

    // ── Player action ──
    match payload.action.as_str() {
        "attack" => {
            let dmg = calc_damage(player.atk, enemy.def, &mut rng);
            enemy_hp -= dmg;
            log.push(format!("🗡️ [{}] 공격! {} 피해!", player.title, dmg));
        }
        "skill" => {
            if lock.skill_used {
                log.push("⚠️ 스킬은 전투당 1회만 사용 가능합니다!".to_string());
            } else {
                lock.skill_used = true;
                let dmg = ((player.atk as f32) * player.skill_multiplier) as i32;
                let actual = (dmg - enemy.def / 2).max(1);
                enemy_hp -= actual;

                // Light element: heal
                if player.element == "Light" {
                    player_hp = (player_hp + 15).min(lock.battle.player_max_hp);
                    log.push(format!("✨ [{}] {} 발동! {} 피해 + 15 HP 회복!", player.title, player.skill_name, actual));
                } else {
                    log.push(format!("✨ [{}] {} 발동! {} 피해!", player.title, player.skill_name, actual));
                }
            }
        }
        "defend" => {
            // Defending gives a temporary shield (simulated by reducing next enemy attack)
            log.push(format!("🛡️ [{}] 방어 태세! 다음 피해 50% 감소.", player.title));
        }
        _ => {}
    }

    // ── Enemy action (AI) ──
    if enemy_hp > 0 {
        let action_roll: f32 = rng.gen_range(0.0..1.0);
        let is_player_defending = payload.action == "defend";
        let def_modifier = if is_player_defending { 0.5 } else { 1.0 };

        if action_roll < 0.2 && turn > 1 {
            // Enemy skill
            let dmg = (((enemy.atk as f32) * enemy.skill_multiplier) as i32 - player.def / 2).max(1);
            let actual = ((dmg as f32) * def_modifier) as i32;
            player_hp -= actual;
            log.push(format!("💥 [{}] {} 발동! {} 피해!", enemy.title, enemy.skill_name, actual));
        } else {
            let dmg = calc_damage(enemy.atk, player.def, &mut rng);
            let actual = ((dmg as f32) * def_modifier) as i32;
            player_hp -= actual;
            log.push(format!("👹 [{}] 공격! {} 피해!", enemy.title, actual));
        }
    }

    player_hp = player_hp.max(0);
    enemy_hp = enemy_hp.max(0);

    let mut battle_over = false;
    let mut player_won = None;

    if player_hp == 0 {
        battle_over = true;
        player_won = Some(false);
        log.push("💀 패배했습니다...".to_string());
        gems_earned = 10;
    } else if enemy_hp == 0 {
        battle_over = true;
        player_won = Some(true);
        log.push("🏆 승리! 보상 젬을 획득했습니다!".to_string());
        // Reward based on enemy rarity
        gems_earned = match enemy.rarity.as_str() {
            "SR"  => 80,
            "R"   => 50,
            _     => 30,
        };
    }

    lock.gems += gems_earned;

    lock.battle = BattleState {
        player_card: Some(player),
        enemy_card: Some(enemy),
        player_hp,
        enemy_hp,
        player_max_hp: lock.battle.player_max_hp,
        enemy_max_hp: lock.battle.enemy_max_hp,
        turn: turn + 1,
        log,
        battle_over,
        player_won,
    };

    let gems = lock.gems;
    let battle = lock.battle.clone();

    Json(BattleActionResponse { battle, gems_earned, gems })
}

fn calc_damage(atk: i32, def: i32, rng: &mut impl Rng) -> i32 {
    let base = (atk - def / 2).max(5);
    let variance: f32 = rng.gen_range(0.85..1.15);
    ((base as f32) * variance) as i32
}
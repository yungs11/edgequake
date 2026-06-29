# edgequake (kb-pipeline 전용) 기동 매뉴얼 — 핸드오프

이 문서는 **Rust 빌드 환경 없이** kb-pipeline 의 전용 edgequake 를 띄우는 절차다.
edgequake 본체는 미리 컴파일된 Docker 이미지(`edgequake-kbp-amd64.tar`)로 전달되며,
원본 `start_dedicated_edgequake.sh`(로컬 바이너리 실행)를 대체한다.

> ⚠️ 이 이미지에는 kb-pipeline 용 소스 수정(AGE GIN 인덱스 제거, `kv.rs`)이
> 반영돼 있다. 동일 이미지를 그대로 쓰면 된다 — 별도 `cargo build` 불필요.

---

## 0. 전제 조건

- **Docker / Docker Compose v2** 설치 (`docker compose version` 으로 확인)
- 아키텍처: **amd64 / x86_64** (이 타르볼은 amd64 단일 빌드)
  - ARM(Apple Silicon/Graviton) 머신이면 이 타르볼은 실행되지 않는다 → 전달자에게 arm64 빌드 요청.
- 외부 접근 가능해야 하는 엔드포인트
  - `https://openrouter.ai` (추출/질의 LLM)
  - 임베딩 게이트웨이 (기본 `https://litellm.ax-demo.com/v1`, 자체 게이트웨이로 교체 가능)

## 1. 전달받는 파일

| 파일 | 설명 |
|---|---|
| `edgequake-kbp-amd64.tar` | edgequake 본체 이미지 (kv.rs 수정 반영, amd64) |
| `docker-compose.kbp.yml` | pg + edgequake 2서비스 정의 |
| `.env.kbp.example` | API 키 템플릿 |

> `docker-compose.kbp.yml` 과 `.env.kbp.example` 은 레포의
> `edgequake/edgequake/` 에 들어 있다. 타르볼만 별도로 전달받는다.

## 2. 이미지 적재 (1회)

```bash
cd edgequake/edgequake          # compose 파일이 있는 위치
docker load -i /path/to/edgequake-kbp-amd64.tar
docker images edgequake-kbp     # edgequake-kbp:local 확인
```

`eq-pg-kbp` Postgres 이미지(`ghcr.io/raphaelmansuy/edgequake-postgres:latest`)는
compose 가 처음 `up` 할 때 자동으로 pull 한다 (네트워크 필요).

## 3. 키 입력 (1회)

```bash
cp .env.kbp.example .env.kbp
# .env.kbp 를 열어 본인 키 입력:
#   OPENROUTER_API_KEY=sk-or-...
#   LITELLM_API_KEY=sk-...
#   (임베딩 엔드포인트를 바꾸려면 EDGEQUAKE_EMBEDDING_BASE_URL 도)
```

> `.env.kbp` 는 커밋 금지.

## 4. 기동

```bash
docker compose -f docker-compose.kbp.yml --env-file .env.kbp up -d
```

- `eq-pg-kbp` 가 먼저 뜨고 healthy 가 된 뒤에야 `eq-kbp`(edgequake) 가 뜬다
  (`depends_on: condition: service_healthy`).
- edgequake 는 부팅 시 DB 마이그레이션을 자체 수행한다(바이너리에 임베드).

## 5. 정상 확인

```bash
docker compose -f docker-compose.kbp.yml ps         # 두 서비스 모두 healthy
curl -f http://localhost:8081/health                # edgequake API
docker logs -f eq-kbp                                # 부팅/요청 로그
```

`/health` 가 200 이면 기동 성공. facade 등 호스트 서비스는 기존처럼
`http://localhost:8081` 로 edgequake 에 접근한다.

## 6. 운영 명령

```bash
# 정지
docker compose -f docker-compose.kbp.yml down

# 정지 + DB 볼륨까지 삭제(초기화)  ※ 데이터 날아감
docker compose -f docker-compose.kbp.yml down -v

# 코드/이미지 갱신 후 재기동
docker load -i edgequake-kbp-amd64.tar              # 새 이미지 적재
docker compose -f docker-compose.kbp.yml --env-file .env.kbp up -d
```

---

## 절대 바꾸지 말 것 (kb-pipeline 불변식)

compose 에 이미 박혀 있다. 임의 수정 시 적재/검색이 깨진다.

| 항목 | 값 | 깨지는 증상 |
|---|---|---|
| `EDGEQUAKE_CHUNKER` | `passthrough` | `adaptive` 로 바꾸면 재청킹 → HTTP 422 적재 실패 |
| 임베딩 모델/차원 | `bge-m3` / `1024` | 청킹·적재·검색 3구간 차원 불일치 |
| `DATABASE_URL` | `search_path=public` 핀 **금지** | AGE graphid 연산자 깨짐 → 그래프 쿼리 500 |
| LLM provider | `openrouter` (qwen) | COMPAT-GUARD 가 모델을 gpt-4.1-nano 로 조용히 다운그레이드 |

---

## 트러블슈팅

| 증상 | 원인 / 조치 |
|---|---|
| `exec format error` | 타르볼 arch 불일치 (amd64 이미지를 arm 머신에서 실행). arm64 빌드 필요. |
| edgequake 가 계속 재시작 | `docker logs eq-kbp` 확인. 보통 DB 연결 실패 → `eq-pg-kbp` healthy 인지 확인. |
| `/health` 200 인데 적재 422 | `EDGEQUAKE_CHUNKER` 가 passthrough 인지 확인. |
| 그래프/문서그래프 쿼리 500 | `DATABASE_URL` 에 `search_path=public` 가 붙었는지 확인 (붙으면 안 됨). |
| LLM 호출 401/403 | `.env.kbp` 의 `OPENROUTER_API_KEY` 확인. |
| 임베딩 호출 실패 | `EDGEQUAKE_EMBEDDING_BASE_URL` 도달성 + `LITELLM_API_KEY` 확인. |

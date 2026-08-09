.PHONY: run dev-db db-bootstrap db-validate db-classify themes static-sync fmt clippy test docker-up docker-down

themes:
	MAVEN_USER_HOME=$${PWD}/target/maven-home ../lorsource-java/mvnw -Dmaven.repo.local=$${PWD}/target/maven-repository -f theme-pom.xml compile

static-sync:
	ORIGINAL_ROOT=$${ORIGINAL_ROOT:-../lorsource-java} ./scripts/sync-java-runtime-assets.sh

dev-db:
	docker compose -f docker-compose.dev.yml up -d postgres
	docker compose -f docker-compose.dev.yml run --rm db-bootstrap bootstrap

db-bootstrap:
	docker compose -f docker-compose.dev.yml run --rm db-bootstrap bootstrap

db-validate:
	docker compose -f docker-compose.dev.yml run --rm db-bootstrap validate

db-classify:
	docker compose -f docker-compose.dev.yml run --rm db-bootstrap classify

run:
	cargo run

fmt:
	cargo fmt --all

clippy:
	cargo clippy --all-targets --all-features -- -D warnings

test:
	cargo test

docker-up:
	docker compose up --build

docker-down:
	docker compose down

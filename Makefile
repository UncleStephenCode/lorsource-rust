.PHONY: run dev-db migrate themes fmt clippy test docker-up docker-down

themes:
	MAVEN_USER_HOME=$${PWD}/target/maven-home ../lorsource-java/mvnw -Dmaven.repo.local=$${PWD}/target/maven-repository -f theme-pom.xml compile

dev-db:
	docker compose -f docker-compose.dev.yml up -d postgres

migrate:
	DATABASE_URL=$${DATABASE_URL:-postgres://lor:lor@localhost:5432/lor} cargo run --bin lorsource-rust

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

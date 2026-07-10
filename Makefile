.PHONY: run dev-db migrate fmt clippy test docker-up docker-down

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

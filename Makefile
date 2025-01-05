generate:
	bash -c "cd demo && npm install"
	cargo install wasm-pack
	wasm-pack build demo/wasm -d ../lib
	cp -r openapi demo/openapi
	echo * > demo/openapi/.gitignore
run:
	bash -c "cd demo && npm run dev"
deploy:
	node -v
	npm -v
	bash -c "cd demo && npm run build"
tree:
	cargo tree
crate-next-app:
	npx create-next-app@latest demo --no-tailwind --no-turbopack --yes
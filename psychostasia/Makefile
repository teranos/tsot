.PHONY: run test assets push-assets check-account

# Media lives in S3, never in git (infra/psychostasia.tf); assets.sha256
# is the record of exactly which files the piece runs on.
ACCOUNT := 548351057127
BUCKET := s3://psychostasia-assets-$(ACCOUNT)

run:
	bun run server.ts

test:
	bun test

# The bucket is private to one account: refuse to run with credentials
# for any other, and say which account they are for.
check-account:
	@account=$$(aws sts get-caller-identity --query Account --output text); \
	if [ "$$account" != "$(ACCOUNT)" ]; then \
		echo "error: AWS credentials are for account '$$account'; the media bucket is in $(ACCOUNT). Set AWS_PROFILE to that account."; \
		exit 1; \
	fi

# Pull the media and fail if any file differs from assets.sha256.
assets: check-account
	aws s3 sync $(BUCKET) assets
	shasum -a 256 -c assets.sha256

# Refuse to upload anything that differs from assets.sha256.
push-assets: check-account
	shasum -a 256 -c assets.sha256
	aws s3 sync assets $(BUCKET)

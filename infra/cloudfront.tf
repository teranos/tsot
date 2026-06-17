# Two distributions:
#
# - `static`  — serves the game bundle out of S3. Standard cached
#   static site. ACM cert on roam.sbvh.nl.
#
# - `relay`   — fronts the libp2p relay's WebSocket. Cache disabled,
#   all viewer headers forwarded so the WS Upgrade handshake survives.
#   ACM cert on relay.sbvh.nl. Origin protocol is HTTP-only because
#   the relay doesn't terminate TLS itself.

# ----- static (game bundle) -----

resource "aws_cloudfront_distribution" "static" {
  enabled             = true
  comment             = "roam game static bundle"
  default_root_object = "index.html"
  price_class         = "PriceClass_100" # NA + EU; cheapest tier

  aliases = [local.game_fqdn]

  origin {
    domain_name              = aws_s3_bucket.static.bucket_regional_domain_name
    origin_id                = "s3-static"
    origin_access_control_id = aws_cloudfront_origin_access_control.static.id
  }

  default_cache_behavior {
    target_origin_id       = "s3-static"
    viewer_protocol_policy = "redirect-to-https"

    allowed_methods = ["GET", "HEAD"]
    cached_methods  = ["GET", "HEAD"]

    # AWS-managed CachingOptimized policy (auto Gzip, sane TTLs).
    cache_policy_id = data.aws_cloudfront_cache_policy.caching_optimized.id

    compress = true
  }

  restrictions {
    geo_restriction {
      restriction_type = "none"
    }
  }

  viewer_certificate {
    acm_certificate_arn      = aws_acm_certificate_validation.game.certificate_arn
    ssl_support_method       = "sni-only"
    minimum_protocol_version = "TLSv1.2_2021"
  }
}

# ----- relay (WebSocket) -----
#
# `relay_origin_domain` is parameterized — points at a placeholder
# until Lightsail is up and a DNS record for `origin-relay.sbvh.nl`
# (or whatever the operator chooses) is set to the box's static IP.
# At that point: `tofu apply -var relay_origin_domain=...` swaps it in.

resource "aws_cloudfront_distribution" "relay" {
  enabled         = true
  comment         = "roam libp2p relay (WebSocket)"
  price_class     = "PriceClass_100"

  aliases = [local.relay_fqdn]

  origin {
    domain_name = var.relay_origin_domain
    origin_id   = "lightsail-relay"

    custom_origin_config {
      http_port                = var.relay_origin_port
      https_port               = 443 # unused — protocol policy is http-only
      origin_protocol_policy   = "http-only"
      origin_ssl_protocols     = ["TLSv1.2"]
      origin_read_timeout      = 60
      origin_keepalive_timeout = 60
    }
  }

  default_cache_behavior {
    target_origin_id       = "lightsail-relay"
    viewer_protocol_policy = "https-only"

    allowed_methods = ["GET", "HEAD", "OPTIONS", "PUT", "POST", "PATCH", "DELETE"]
    cached_methods  = ["GET", "HEAD"]

    # AWS-managed CachingDisabled — required for WebSocket; cache
    # behavior must not try to memoize the upgrade.
    cache_policy_id = data.aws_cloudfront_cache_policy.caching_disabled.id

    # AWS-managed AllViewer origin request policy — forwards every
    # viewer header to the origin, including `Upgrade`, `Connection`,
    # and `Sec-WebSocket-*` which the upgrade handshake needs.
    origin_request_policy_id = data.aws_cloudfront_origin_request_policy.all_viewer.id

    compress = false
  }

  restrictions {
    geo_restriction {
      restriction_type = "none"
    }
  }

  viewer_certificate {
    acm_certificate_arn      = aws_acm_certificate_validation.relay.certificate_arn
    ssl_support_method       = "sni-only"
    minimum_protocol_version = "TLSv1.2_2021"
  }
}

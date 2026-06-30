
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


# rave.sbvh.nl — static site for the Bevy crate at `rave/`.
# Mirrors universe.tf — separate bucket + distribution + cert + role
# so the blast radius is scoped to rave only.

# ----- S3 bucket -----

resource "aws_s3_bucket" "rave_static" {
  bucket        = var.rave_bucket_name
  force_destroy = true
}

resource "aws_s3_bucket_public_access_block" "rave_static" {
  bucket = aws_s3_bucket.rave_static.id

  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_cloudfront_origin_access_control" "rave_static" {
  name                              = "${var.rave_bucket_name}-oac"
  description                       = "OAC for rave static bucket"
  origin_access_control_origin_type = "s3"
  signing_behavior                  = "always"
  signing_protocol                  = "sigv4"
}

resource "aws_s3_bucket_policy" "rave_static_cf_read" {
  bucket = aws_s3_bucket.rave_static.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Sid       = "AllowCloudFrontServicePrincipalRead"
        Effect    = "Allow"
        Principal = { Service = "cloudfront.amazonaws.com" }
        Action    = ["s3:GetObject"]
        Resource  = "${aws_s3_bucket.rave_static.arn}/*"
        Condition = {
          StringEquals = {
            "AWS:SourceArn" = aws_cloudfront_distribution.rave_static.arn
          }
        }
      }
    ]
  })
}

# ----- ACM certificate (us-east-1 — CloudFront requirement) -----

resource "aws_acm_certificate" "rave" {
  provider = aws.us_east_1

  domain_name       = local.rave_fqdn
  validation_method = "DNS"

  lifecycle {
    create_before_destroy = true
  }
}

resource "aws_route53_record" "rave_cert_validation" {
  for_each = {
    for dvo in aws_acm_certificate.rave.domain_validation_options : dvo.domain_name => {
      name   = dvo.resource_record_name
      record = dvo.resource_record_value
      type   = dvo.resource_record_type
    }
  }

  zone_id         = data.aws_route53_zone.root.zone_id
  name            = each.value.name
  type            = each.value.type
  records         = [each.value.record]
  ttl             = 60
  allow_overwrite = true
}

resource "aws_acm_certificate_validation" "rave" {
  provider = aws.us_east_1

  certificate_arn         = aws_acm_certificate.rave.arn
  validation_record_fqdns = [for r in aws_route53_record.rave_cert_validation : r.fqdn]
}

# ----- CloudFront distribution -----

resource "aws_cloudfront_distribution" "rave_static" {
  enabled             = true
  comment             = "rave (Bevy + libp2p) static bundle"
  default_root_object = "index.html"
  price_class         = "PriceClass_100"

  aliases = [local.rave_fqdn]

  origin {
    domain_name              = aws_s3_bucket.rave_static.bucket_regional_domain_name
    origin_id                = "s3-rave-static"
    origin_access_control_id = aws_cloudfront_origin_access_control.rave_static.id
  }

  default_cache_behavior {
    target_origin_id       = "s3-rave-static"
    viewer_protocol_policy = "redirect-to-https"

    allowed_methods = ["GET", "HEAD"]
    cached_methods  = ["GET", "HEAD"]

    cache_policy_id = data.aws_cloudfront_cache_policy.caching_optimized.id

    compress = true
  }

  restrictions {
    geo_restriction {
      restriction_type = "none"
    }
  }

  viewer_certificate {
    acm_certificate_arn      = aws_acm_certificate_validation.rave.certificate_arn
    ssl_support_method       = "sni-only"
    minimum_protocol_version = "TLSv1.2_2021"
  }
}

# ----- DNS -----

resource "aws_route53_record" "rave" {
  zone_id = data.aws_route53_zone.root.zone_id
  name    = local.rave_fqdn
  type    = "A"

  alias {
    name                   = aws_cloudfront_distribution.rave_static.domain_name
    zone_id                = aws_cloudfront_distribution.rave_static.hosted_zone_id
    evaluate_target_health = false
  }
}

# ----- GitHub Actions deploy role -----

resource "aws_iam_role" "rave_github_deploy" {
  name = "rave-github-deploy"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect = "Allow"
      Principal = {
        Federated = aws_iam_openid_connect_provider.github.arn
      }
      Action = "sts:AssumeRoleWithWebIdentity"
      Condition = {
        StringEquals = {
          "token.actions.githubusercontent.com:aud" = "sts.amazonaws.com"
        }
        StringLike = {
          "token.actions.githubusercontent.com:sub" = [
            for ref in var.rave_deploy_refs : "repo:${var.github_repo}:ref:${ref}"
          ]
        }
      }
    }]
  })
}

resource "aws_iam_role_policy" "rave_github_deploy" {
  name = "deploy-permissions"
  role = aws_iam_role.rave_github_deploy.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Sid    = "RaveBucketWrite"
        Effect = "Allow"
        Action = [
          "s3:PutObject",
          "s3:DeleteObject",
          "s3:GetObject",
          "s3:ListBucket",
        ]
        Resource = [
          aws_s3_bucket.rave_static.arn,
          "${aws_s3_bucket.rave_static.arn}/*",
        ]
      },
      {
        Sid    = "RaveCloudFrontInvalidate"
        Effect = "Allow"
        Action = [
          "cloudfront:CreateInvalidation",
          "cloudfront:GetInvalidation",
        ]
        Resource = aws_cloudfront_distribution.rave_static.arn
      },
    ]
  })
}

output "rave_github_deploy_role_arn" {
  description = "Role ARN the rave deploy workflow assumes via OIDC."
  value       = aws_iam_role.rave_github_deploy.arn
}

output "rave_distribution_id" {
  description = "CloudFront distribution ID for rave — used by the workflow for invalidations."
  value       = aws_cloudfront_distribution.rave_static.id
}

output "rave_fqdn" {
  description = "Public hostname rave is served at."
  value       = local.rave_fqdn
}

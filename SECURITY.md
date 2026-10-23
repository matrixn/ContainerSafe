# Security Policy

ContainerSafe is an **early-stage development prototype**, not yet approved for production backup or recovery use.

## Security reports

Please **do not post vulnerabilities, passwords, SSH keys, live Docker output, `.env` files, or unredacted inventory** in public GitHub issues.

If private vulnerability reporting is enabled for the repository, use GitHub's **Report a vulnerability** option under the Security tab. If it is not enabled, open a public issue containing only a request for a private reporting channel, without vulnerability details or sensitive data.

## Supported versions

There are no formally supported production releases or security-maintenance guarantees at this stage.

## Safe testing

- Use disposable Docker stacks and test data.
- Do not mount or expose Docker's privileged socket outside a secured host.
- Treat Docker API access as privileged.
- Keep backup repository passwords out of process arguments and source control.
- Run restoration tests in isolated locations, never over production data.
- Maintain your existing independent backups.

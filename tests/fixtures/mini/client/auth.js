export class AuthenticationClient {
  login(credentials) {
    return this.exchangeCredential(credentials);
  }

  exchangeCredential(credentials) {
    return fetch("/sessions", { method: "POST", body: JSON.stringify(credentials) });
  }
}

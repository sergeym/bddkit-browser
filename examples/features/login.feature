Feature: signing in through the browser

  Scenario: a wrong password stays on the login page
    Given I am on "/login"
    And I fill in "E-mail" with "ann@example.test"
    And I fill in "Password" with "wrong"
    When I press "Sign in"
    Then I should be on "/login?error=1"
    And the page should contain "Wrong e-mail or password"

  Scenario: the right password reaches the dashboard
    Given I am on "/login"
    And I fill in "email" with "ann@example.test"
    And I fill in "password" with "secret"
    When I press "Sign in"
    Then I should be on "/dashboard"
    And the page title should be "Dashboard — bddkit demo"
    And the "h1" element should contain "Welcome, ann@example.test"

  Scenario: the previous scenario's login does not survive the reset
    Given I am on "/dashboard"
    Then I should be on "/login"

Feature: the browser console is part of the evidence

  Scenario: a clean page logs nothing
    Given I am on "/login"
    Then the browser console should have no errors
    When I dump the browser console
    And I dump the network log
